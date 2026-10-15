import 'dart:async';
import 'dart:collection';
import 'dart:io' show Platform;
import 'dart:isolate';

/// Work executed inside a pooled worker isolate.
///
/// Task instances are sent to the worker as-is (workers are spawned with
/// [Isolate.spawn] and share the caller's isolate group), so every field must
/// be sendable. [run] executes on the worker isolate; top-level state it
/// touches (for example cached FFI bindings) is per worker and survives
/// between tasks for as long as the worker lives.
abstract class WorkerTask<R> {
  const WorkerTask();

  /// Executes the task on the worker isolate.
  FutureOr<R> run(WorkerTaskContext context);
}

/// A task result that owns resources outside the Dart heap (for example
/// native buffers referenced by address).
///
/// The pool calls [disposeUndelivered] when such a result cannot reach the
/// caller — the worker failed to send it, or it arrived for a job that had
/// already completed — so those resources are not leaked. A delivered result
/// is owned by the caller from then on.
abstract interface class WorkerDisposable {
  /// Releases the resources of a result that was never delivered.
  void disposeUndelivered();
}

/// Worker-side channel between a running [WorkerTask] and its caller.
abstract interface class WorkerTaskContext {
  /// Sends [event] to the caller's [WorkerEventHandler]. Events, the final
  /// result and failures travel over one port, so they arrive in order.
  void send(Object? event);

  /// Receives messages sent with [WorkerJob.send] or replies from the
  /// caller's [WorkerEventHandler]. Messages that arrive before a handler is
  /// installed are buffered and delivered when it is set.
  set onMessage(void Function(Object? message)? handler);
}

/// Main-side handler for events sent by a task with [WorkerTaskContext.send].
///
/// [reply] delivers a message to the task's [WorkerTaskContext.onMessage].
typedef WorkerEventHandler = void Function(
  Object? event,
  void Function(Object? message) reply,
);

/// Handle for a task submitted to a [WorkerPool].
class WorkerJob<R> {
  WorkerJob._(this._job);

  final _PoolJob<R> _job;

  /// Completes with the task's result, or with the error it threw.
  Future<R> get result => _job.completer.future;

  /// Sends [message] to the running task. Messages sent while the job is
  /// still queued are delivered as soon as it starts.
  void send(Object? message) => _job.send(message);
}

/// A small pool of long-lived worker isolates.
///
/// * Workers are spawned lazily, up to [maxWorkers], and each runs one task
///   at a time; extra tasks wait in a FIFO queue.
/// * A worker that stays idle for [idleTimeout] retires on its own, so no
///   resources are held while the library is unused.
/// * Idle workers never keep the owning isolate alive, so they do not delay
///   process exit.
/// * If a worker dies unexpectedly the task it was running fails and the pool
///   replaces the worker on demand.
class WorkerPool {
  WorkerPool({
    int? maxWorkers,
    this.idleTimeout = defaultIdleTimeout,
    this.debugName = 'ironpress-worker',
  }) : maxWorkers = maxWorkers ?? defaultMaxWorkers() {
    if (this.maxWorkers < 1) {
      throw ArgumentError.value(this.maxWorkers, 'maxWorkers', 'must be >= 1');
    }
  }

  /// How long an idle worker waits for new work before it exits.
  static const Duration defaultIdleTimeout = Duration(seconds: 10);

  /// Concurrency limit derived from the number of CPU cores.
  static int defaultMaxWorkers() {
    final cores = Platform.numberOfProcessors;
    if (cores < 2) {
      return 2;
    }
    return cores > 16 ? 16 : cores;
  }

  /// Pool used by the public `Ironpress` API (internal; replaceable in tests).
  static WorkerPool shared = WorkerPool();

  /// Maximum number of worker isolates alive at once.
  final int maxWorkers;

  /// Idle time after which a worker exits.
  final Duration idleTimeout;

  /// Debug name prefix for spawned isolates.
  final String debugName;

  final List<_WorkerHandle> _workers = <_WorkerHandle>[];
  final Queue<_PoolJob<Object?>> _queue = Queue<_PoolJob<Object?>>();
  int _nextJobId = 0;
  int _spawnCount = 0;

  /// Number of live (starting, idle or busy) workers.
  int get debugWorkerCount => _workers.length;

  /// Total number of workers spawned by this pool so far.
  int get debugSpawnCount => _spawnCount;

  /// Number of tasks waiting for a free worker.
  int get debugQueuedCount => _queue.length;

  /// Runs [task] on a worker and returns its result.
  Future<R> run<R>(WorkerTask<R> task) => submit(task).result;

  /// Submits [task] and returns a handle for messaging it.
  ///
  /// [onEvent] runs in the caller's zone for every event the task sends.
  /// [crashError] maps an unexpected worker death to the error reported to
  /// the caller (default: [RemoteError]).
  WorkerJob<R> submit<R>(
    WorkerTask<R> task, {
    WorkerEventHandler? onEvent,
    Object Function(String details)? crashError,
  }) {
    final job = _PoolJob<R>(
      _nextJobId++,
      task,
      onEvent,
      crashError,
      Zone.current,
    );
    _queue.add(job);
    _pump();
    return WorkerJob<R>._(job);
  }

  void _pump() {
    while (_queue.isNotEmpty) {
      final worker = _firstIdleWorker();
      if (worker == null) {
        break;
      }
      _assign(worker, _queue.removeFirst());
    }

    var starting = 0;
    for (final worker in _workers) {
      if (worker.command == null) {
        starting++;
      }
    }
    var deficit = _queue.length - starting;
    while (deficit > 0 && _workers.length < maxWorkers) {
      _spawn();
      deficit--;
    }
  }

  _WorkerHandle? _firstIdleWorker() {
    for (final worker in _workers) {
      if (worker.command != null && worker.current == null) {
        return worker;
      }
    }
    return null;
  }

  void _assign(_WorkerHandle worker, _PoolJob<Object?> job) {
    worker.current = job;
    job.worker = worker;
    _updateKeepAlive(worker);
    try {
      worker.command!.send(
        _RunJob(job.id, job.task, List<Object?>.of(job.outbox)),
      );
    } catch (error, stackTrace) {
      // The task (or a replayed message) is not sendable.
      worker.current = null;
      job.worker = null;
      _updateKeepAlive(worker);
      job.completeError(error, stackTrace);
    }
  }

  void _spawn() {
    final worker = _WorkerHandle(RawReceivePort(null, '$debugName-port'));
    worker.port.handler = (Object? message) => _onMessage(worker, message);
    _workers.add(worker);
    final index = _spawnCount++;
    Isolate.spawn<_WorkerBootstrap>(
      _workerMain,
      _WorkerBootstrap(worker.port.sendPort, idleTimeout),
      onError: worker.port.sendPort,
      onExit: worker.port.sendPort,
      errorsAreFatal: true,
      debugName: '$debugName-$index',
    ).then<void>(
      (_) {},
      onError: (Object error, StackTrace stackTrace) {
        worker.port.close();
        _workers.remove(worker);
        if (_workers.isEmpty) {
          _failQueued(error, stackTrace);
        }
      },
    );
  }

  void _failQueued(Object error, StackTrace stackTrace) {
    while (_queue.isNotEmpty) {
      _queue.removeFirst().completeError(error, stackTrace);
    }
  }

  void _onMessage(_WorkerHandle worker, Object? message) {
    if (message is _JobEvent) {
      final job = worker.current;
      if (job == null || job.id != message.jobId) {
        return;
      }
      job.handleEvent(message.event, (Object? reply) {
        if (identical(worker.current, job)) {
          worker.command?.send(_JobInbox(job.id, reply));
        }
      });
      return;
    }

    if (message is _JobDone) {
      final job = _takeJob(worker, message.jobId);
      if (job == null || !job.complete(message.value)) {
        _disposeUndelivered(message.value);
      }
      _pump();
      return;
    }

    if (message is _JobFailed) {
      _takeJob(worker, message.jobId)?.completeError(
        message.error,
        StackTrace.fromString(message.stackTrace),
      );
      _pump();
      return;
    }

    if (message is _WorkerReady) {
      worker.command = message.commandPort;
      _updateKeepAlive(worker);
      _pump();
      return;
    }

    if (message is _WorkerRetiring) {
      // The worker went idle and is exiting. A job dispatched while its idle
      // timer fired was never started, so it is safe to run it elsewhere.
      _workers.remove(worker);
      worker.retired = true;
      final job = worker.current;
      worker.current = null;
      _updateKeepAlive(worker);
      if (job != null) {
        job.worker = null;
        _queue.addFirst(job);
      }
      _pump();
      return;
    }

    if (message == null) {
      // `onExit` notification (sent as null): the worker isolate is gone.
      worker.port.close();
      final wasReady = worker.command != null || worker.retired;
      _workers.remove(worker);
      final job = worker.current;
      worker.current = null;
      final details = worker.lastError ?? 'Computation ended without result';
      if (job != null) {
        job.crash(details, worker.lastErrorStack);
      }
      if (!wasReady && _workers.isEmpty) {
        // A worker died during start-up; avoid a respawn loop.
        _failQueued(
          RemoteError(details, worker.lastErrorStack ?? ''),
          StackTrace.empty,
        );
      }
      _pump();
      return;
    }

    if (message is List && message.length == 2) {
      // Uncaught error reported by `onError`; the exit message follows.
      worker.lastError = '${message[0]}';
      worker.lastErrorStack = message[1] as String?;
    }
  }

  _PoolJob<Object?>? _takeJob(_WorkerHandle worker, int jobId) {
    final job = worker.current;
    if (job == null || job.id != jobId) {
      return null;
    }
    worker.current = null;
    job.worker = null;
    _updateKeepAlive(worker);
    return job;
  }

  void _updateKeepAlive(_WorkerHandle worker) {
    // Keep the owner alive only while it waits on this worker.
    worker.port.keepIsolateAlive =
        !worker.retired && (worker.command == null || worker.current != null);
  }
}

// ─── Main-side bookkeeping ─────────────────────────────────────────────────

class _WorkerHandle {
  _WorkerHandle(this.port);

  final RawReceivePort port;
  SendPort? command;
  _PoolJob<Object?>? current;
  bool retired = false;
  String? lastError;
  String? lastErrorStack;
}

class _PoolJob<R> {
  _PoolJob(this.id, this.task, this.onEvent, this.crashError, this.zone);

  final int id;
  final WorkerTask<R> task;
  final WorkerEventHandler? onEvent;
  final Object Function(String details)? crashError;
  final Zone zone;
  final Completer<R> completer = Completer<R>();
  final List<Object?> outbox = <Object?>[];
  _WorkerHandle? worker;

  void send(Object? message) {
    if (completer.isCompleted) {
      return;
    }
    outbox.add(message);
    final command = worker?.command;
    if (command != null) {
      command.send(_JobInbox(id, message));
    }
  }

  void handleEvent(Object? event, void Function(Object? message) reply) {
    final handler = onEvent;
    if (handler == null || completer.isCompleted) {
      return;
    }
    zone.runBinaryGuarded(handler, event, reply);
  }

  /// Completes the job with [value]; returns false if it already completed.
  bool complete(Object? value) {
    if (completer.isCompleted) {
      return false;
    }
    completer.complete(value as R);
    return true;
  }

  void completeError(Object error, StackTrace stackTrace) {
    if (!completer.isCompleted) {
      completer.completeError(error, stackTrace);
    }
  }

  void crash(String details, String? stackTrace) {
    final mapped =
        crashError?.call(details) ?? RemoteError(details, stackTrace ?? '');
    completeError(mapped, StackTrace.fromString(stackTrace ?? ''));
  }
}

void _disposeUndelivered(Object? value) {
  if (value is WorkerDisposable) {
    try {
      value.disposeUndelivered();
    } catch (_) {
      // Best effort: never let cleanup break the pool.
    }
  }
}

// ─── Protocol ──────────────────────────────────────────────────────────────

class _WorkerBootstrap {
  const _WorkerBootstrap(this.mainPort, this.idleTimeout);

  final SendPort mainPort;
  final Duration idleTimeout;
}

class _WorkerReady {
  const _WorkerReady(this.commandPort);

  final SendPort commandPort;
}

class _WorkerRetiring {
  const _WorkerRetiring();
}

class _RunJob {
  const _RunJob(this.jobId, this.task, this.replay);

  final int jobId;
  final WorkerTask<Object?> task;
  final List<Object?> replay;
}

class _JobInbox {
  const _JobInbox(this.jobId, this.message);

  final int jobId;
  final Object? message;
}

class _JobEvent {
  const _JobEvent(this.jobId, this.event);

  final int jobId;
  final Object? event;
}

class _JobDone {
  const _JobDone(this.jobId, this.value);

  final int jobId;
  final Object? value;
}

class _JobFailed {
  const _JobFailed(this.jobId, this.error, this.stackTrace);

  final int jobId;
  final Object error;
  final String stackTrace;
}

// ─── Worker side ───────────────────────────────────────────────────────────

class _WorkerContext implements WorkerTaskContext {
  _WorkerContext(this.jobId, this._out);

  final int jobId;
  final SendPort _out;
  final List<Object?> _buffer = <Object?>[];
  void Function(Object? message)? _handler;
  bool _closed = false;

  @override
  void send(Object? event) {
    if (!_closed) {
      _out.send(_JobEvent(jobId, event));
    }
  }

  @override
  set onMessage(void Function(Object? message)? handler) {
    _handler = handler;
    if (handler == null) {
      return;
    }
    while (_buffer.isNotEmpty && identical(_handler, handler)) {
      handler(_buffer.removeAt(0));
    }
  }

  void deliver(Object? message) {
    if (_closed) {
      return;
    }
    final handler = _handler;
    if (handler == null) {
      _buffer.add(message);
    } else {
      handler(message);
    }
  }

  void close() {
    _closed = true;
    _handler = null;
    _buffer.clear();
  }
}

void _workerMain(_WorkerBootstrap bootstrap) {
  final out = bootstrap.mainPort;
  final commands = RawReceivePort(null, 'ironpress-worker-commands');
  Timer? idleTimer;
  _WorkerContext? active;

  void armIdleTimer() {
    idleTimer?.cancel();
    idleTimer = Timer(bootstrap.idleTimeout, () {
      // Only reachable with no active job: starting a job cancels the timer.
      out.send(const _WorkerRetiring());
      commands.close();
    });
  }

  Future<void> runJob(_RunJob job) async {
    final context = _WorkerContext(job.jobId, out);
    active = context;
    for (final message in job.replay) {
      context.deliver(message);
    }
    try {
      final value = await job.task.run(context);
      try {
        out.send(_JobDone(job.jobId, value));
      } catch (error, stackTrace) {
        _disposeUndelivered(value);
        out.send(
          _JobFailed(
            job.jobId,
            RemoteError(error.toString(), stackTrace.toString()),
            stackTrace.toString(),
          ),
        );
      }
    } catch (error, stackTrace) {
      try {
        out.send(_JobFailed(job.jobId, error, stackTrace.toString()));
      } catch (_) {
        out.send(
          _JobFailed(
            job.jobId,
            RemoteError(error.toString(), stackTrace.toString()),
            stackTrace.toString(),
          ),
        );
      }
    } finally {
      context.close();
      if (identical(active, context)) {
        active = null;
      }
      armIdleTimer();
    }
  }

  commands.handler = (Object? message) {
    if (message is _RunJob) {
      idleTimer?.cancel();
      idleTimer = null;
      unawaited(runJob(message));
      return;
    }
    if (message is _JobInbox) {
      final context = active;
      if (context != null && context.jobId == message.jobId) {
        context.deliver(message.message);
      }
    }
  };

  out.send(_WorkerReady(commands.sendPort));
  armIdleTimer();
}
