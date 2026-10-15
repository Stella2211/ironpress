import 'dart:async';
import 'dart:isolate';

import 'package:flutter_test/flutter_test.dart';
import 'package:ironpress/src/worker_pool.dart';

/// Per-isolate counter: proves that consecutive tasks share one worker.
int _workerLocalCounter = 0;

class _ValueTask extends WorkerTask<int> {
  const _ValueTask(this.value);

  final int value;

  @override
  int run(WorkerTaskContext context) => value * 2;
}

class _CounterTask extends WorkerTask<int> {
  const _CounterTask();

  @override
  int run(WorkerTaskContext context) => ++_workerLocalCounter;
}

class _DelayTask extends WorkerTask<int> {
  const _DelayTask(this.delay, this.value);

  final Duration delay;
  final int value;

  @override
  Future<int> run(WorkerTaskContext context) async {
    await Future<void>.delayed(delay);
    return value;
  }
}

class _CustomError implements Exception {
  const _CustomError(this.code);

  final int code;
}

class _ThrowingTask extends WorkerTask<int> {
  const _ThrowingTask(this.error);

  final Object error;

  @override
  int run(WorkerTaskContext context) => throw error;
}

/// Kills its worker with an uncaught asynchronous error.
class _CrashTask extends WorkerTask<int> {
  const _CrashTask();

  @override
  Future<int> run(WorkerTaskContext context) {
    Timer.run(() => throw StateError('worker exploded'));
    return Completer<int>().future;
  }
}

class _UnsendableResultTask extends WorkerTask<Object> {
  const _UnsendableResultTask();

  @override
  Object run(WorkerTaskContext context) => ReceivePort();
}

/// Result owning an external resource; reports its disposal to [owner].
class _DisposableResult implements WorkerDisposable {
  _DisposableResult(this.owner, {this.unsendable});

  final SendPort owner;
  final Object? unsendable;

  @override
  void disposeUndelivered() => owner.send('disposed');
}

class _UnsendableDisposableTask extends WorkerTask<Object> {
  const _UnsendableDisposableTask(this.owner);

  final SendPort owner;

  @override
  Object run(WorkerTaskContext context) =>
      _DisposableResult(owner, unsendable: ReceivePort());
}

/// Echoes events to the caller and returns after receiving 'stop'.
class _ConversationTask extends WorkerTask<List<Object?>> {
  const _ConversationTask();

  @override
  Future<List<Object?>> run(WorkerTaskContext context) {
    final received = <Object?>[];
    final done = Completer<List<Object?>>();
    context.onMessage = (message) {
      if (message == 'stop') {
        done.complete(received);
        return;
      }
      received.add(message);
      context.send('echo:$message');
    };
    context.send('ready');
    return done.future;
  }
}

/// Owner isolate that uses a pool with a long idle timeout and then returns.
Future<void> _ownerMain(SendPort results) async {
  final pool = WorkerPool(idleTimeout: const Duration(minutes: 1));
  results.send(await pool.run(const _ValueTask(4)));
}

Future<void> _waitUntil(bool Function() condition) async {
  final deadline = DateTime.now().add(const Duration(seconds: 10));
  while (!condition()) {
    if (DateTime.now().isAfter(deadline)) {
      fail('Condition not met before the deadline');
    }
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

void main() {
  group('WorkerPool', () {
    late WorkerPool pool;

    setUp(() {
      pool = WorkerPool(
        maxWorkers: 2,
        idleTimeout: const Duration(milliseconds: 200),
      );
    });

    test('runs a task and returns its result', () async {
      expect(await pool.run(const _ValueTask(21)), 42);
    });

    test('reuses one long-lived worker for sequential tasks', () async {
      final values = <int>[];
      for (var i = 0; i < 5; i++) {
        values.add(await pool.run(const _CounterTask()));
      }
      expect(values, [1, 2, 3, 4, 5]);
      expect(pool.debugSpawnCount, 1);
    });

    test('caps concurrency at maxWorkers and queues the rest', () async {
      final futures = [
        for (var i = 0; i < 6; i++)
          pool.run(_DelayTask(const Duration(milliseconds: 50), i)),
      ];
      expect(pool.debugWorkerCount, lessThanOrEqualTo(2));
      expect(await Future.wait(futures), [0, 1, 2, 3, 4, 5]);
      expect(pool.debugSpawnCount, 2);
    });

    test('propagates task errors with their original type', () async {
      await expectLater(
        pool.run(_ThrowingTask(ArgumentError('bad value'))),
        throwsA(
          isA<ArgumentError>().having((e) => e.message, 'message', 'bad value'),
        ),
      );
      await expectLater(
        pool.run(const _ThrowingTask(_CustomError(7))),
        throwsA(isA<_CustomError>().having((e) => e.code, 'code', 7)),
      );
      // The worker survives task errors.
      expect(await pool.run(const _ValueTask(1)), 2);
      expect(pool.debugSpawnCount, 1);
    });

    test('reports unsendable results as RemoteError', () async {
      await expectLater(
        pool.run(const _UnsendableResultTask()),
        throwsA(isA<RemoteError>()),
      );
      expect(await pool.run(const _ValueTask(2)), 4);
    });

    test('disposes a disposable result that cannot be sent', () async {
      final port = ReceivePort();
      try {
        await expectLater(
          pool.run(_UnsendableDisposableTask(port.sendPort)),
          throwsA(isA<RemoteError>()),
        );
        expect(
          await port.first.timeout(const Duration(seconds: 5)),
          'disposed',
        );
      } finally {
        port.close();
      }
    });

    test(
      'fails the running task when its worker crashes and recovers',
      () async {
        await expectLater(
          pool.run(const _CrashTask()),
          throwsA(
            isA<RemoteError>().having(
              (e) => e.toString(),
              'message',
              contains('worker exploded'),
            ),
          ),
        );
        expect(await pool.run(const _ValueTask(5)), 10);
        expect(pool.debugSpawnCount, 2);
      },
    );

    test('maps crashes through crashError', () async {
      final job = pool.submit(
        const _CrashTask(),
        crashError: (details) => StateError('mapped: $details'),
      );
      await expectLater(
        job.result,
        throwsA(
          isA<StateError>().having(
            (e) => e.message,
            'message',
            contains('worker exploded'),
          ),
        ),
      );
    });

    test('retires idle workers and spawns fresh ones on demand', () async {
      expect(await pool.run(const _CounterTask()), 1);
      expect(pool.debugWorkerCount, 1);

      await _waitUntil(() => pool.debugWorkerCount == 0);

      // A new worker has fresh isolate state.
      expect(await pool.run(const _CounterTask()), 1);
      expect(pool.debugSpawnCount, 2);
    });

    test('never loses work that races with idle retirement', () async {
      final racyPool = WorkerPool(
        maxWorkers: 2,
        idleTimeout: const Duration(milliseconds: 1),
      );
      for (var i = 0; i < 40; i++) {
        expect(await racyPool.run(_ValueTask(i)), i * 2);
        await Future<void>.delayed(Duration(microseconds: 500 * (i % 5)));
      }
    });

    test('delivers events and replies in order', () async {
      final events = <Object?>[];
      final job = pool.submit(
        const _ConversationTask(),
        onEvent: (event, reply) {
          events.add(event);
          if (event == 'ready') {
            reply('a');
            reply('b');
          } else if (event == 'echo:b') {
            reply('stop');
          }
        },
      );
      expect(await job.result, ['a', 'b']);
      expect(events, ['ready', 'echo:a', 'echo:b']);
    });

    test('replays messages sent while a job is still queued', () async {
      final blockers = [
        pool.run(const _DelayTask(Duration(milliseconds: 100), 0)),
        pool.run(const _DelayTask(Duration(milliseconds: 100), 0)),
      ];
      final job = pool.submit(const _ConversationTask());
      job.send('early');
      job.send('stop');
      expect(pool.debugQueuedCount, greaterThanOrEqualTo(1));
      expect(await job.result, ['early']);
      await Future.wait(blockers);
    });

    test('idle workers do not keep the owning isolate alive', () async {
      final results = ReceivePort();
      final exited = ReceivePort();
      final stopwatch = Stopwatch()..start();
      await Isolate.spawn(
        _ownerMain,
        results.sendPort,
        onExit: exited.sendPort,
      );
      expect(await results.first, 8);
      await exited.first.timeout(const Duration(seconds: 10));
      // Far below the one-minute idle timeout of the owner's pool.
      expect(stopwatch.elapsed, lessThan(const Duration(seconds: 10)));
    });

    test('runs onEvent in the caller zone', () async {
      final errors = <Object>[];
      await runZonedGuarded(() async {
        final job = pool.submit(
          const _ConversationTask(),
          onEvent: (event, reply) {
            if (event == 'ready') {
              reply('stop');
              throw StateError('handler failure');
            }
          },
        );
        expect(await job.result, isEmpty);
      }, (error, _) => errors.add(error));
      expect(errors, [isA<StateError>()]);
    });
  });
}
