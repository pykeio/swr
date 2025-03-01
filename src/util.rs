use std::{
	fmt,
	future::Future,
	sync::atomic::{AtomicU64, Ordering},
	time::Duration
};

// Use `tokio`'s `Instant` wrapper in testing since we can 'advance' time with `tokio::time::advance`
#[cfg(test)]
pub type Instant = tokio::time::Instant;
#[cfg(not(test))]
pub type Instant = std::time::Instant;

#[cfg(test)]
pub use tokio::time::{advance, pause};

use crate::runtime::{Runtime, Task};

/// A version of [`Instant`] supporting atomic operations.
pub struct AtomicInstant {
	base: Instant,
	// store offset since (unchanging) base instant in nanoseconds; 64 bits holds an offset of ~584 years
	offset_nanos: AtomicU64
}

impl AtomicInstant {
	#[inline]
	pub fn new(base: Instant) -> AtomicInstant {
		AtomicInstant {
			base,
			offset_nanos: AtomicU64::new(0)
		}
	}

	#[inline]
	pub fn now() -> AtomicInstant {
		AtomicInstant::new(Instant::now())
	}

	pub fn load(&self, order: Ordering) -> Instant {
		let offset_nanos = self.offset_nanos.load(order);
		let secs = offset_nanos / 1_000_000_000;
		let subsec_nanos = (offset_nanos % 1_000_000_000) as u32;
		self.base + Duration::new(secs, subsec_nanos)
	}

	pub fn store(&self, value: Instant, order: Ordering) {
		let offset = value - self.base;
		let offset_nanos = offset.as_secs() * 1_000_000_000 + u64::from(offset.subsec_nanos());
		self.offset_nanos.store(offset_nanos, order);
	}

	#[inline]
	pub fn store_now(&self, order: Ordering) {
		self.store(Instant::now(), order);
	}
}

impl fmt::Debug for AtomicInstant {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.load(Ordering::Relaxed).fmt(f)
	}
}

pub struct TaskSlot<R: Runtime> {
	runtime: R,
	task: Option<R::Task<()>>
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TaskStartMode {
	/// Do not spawn the task if a task is currently running.
	Soft,
	/// If a task is currently running, replace it, but keep the old task running.
	Override,
	/// If a task is currently running, abort it and replace it with the new task.
	Abort
}

impl<R: Runtime> TaskSlot<R> {
	pub fn new(runtime: R) -> Self {
		Self { runtime, task: None }
	}

	pub fn insert<F>(&mut self, mode: TaskStartMode, fut: F) -> bool
	where
		F: Future<Output = ()> + Send + 'static
	{
		match mode {
			TaskStartMode::Soft => {
				if let Some(handle) = &self.task {
					if !handle.is_finished() {
						return false;
					}
				}
			}
			TaskStartMode::Abort => {
				if let Some(handle) = self.task.take() {
					handle.abort();
				}
			}
			TaskStartMode::Override => {}
		}

		self.task.replace(self.runtime.spawn(fut));
		true
	}

	pub fn is_finished(&self) -> bool {
		match &self.task {
			Some(task) => task.is_finished(),
			_ => true
		}
	}

	pub fn abort(&mut self) {
		if let Some(handle) = self.task.take() {
			handle.abort();
		}
	}
}

/// Returns `true` if the time elapsed since `prev_time` exceeds the `throttle_time`.
pub fn throttle(prev_time: Option<&Instant>, throttle_time: Option<&Duration>) -> bool {
	match (prev_time, throttle_time) {
		(Some(prev_time), Some(throttle_time)) => prev_time.elapsed() >= *throttle_time,
		_ => true
	}
}

#[cfg(test)]
mod tests {
	use std::{
		sync::{
			Arc,
			atomic::{AtomicBool, Ordering}
		},
		time::Duration
	};

	use tokio::task::yield_now;

	use super::{AtomicInstant, Instant, TaskSlot, TaskStartMode};
	use crate::runtime::Tokio;

	#[test]
	fn test_atomic_instant() {
		let instant = Arc::new(AtomicInstant::now());
		let mut threads = vec![];
		for _ in 0..4 {
			let instant = Arc::clone(&instant);
			threads.push(std::thread::spawn(move || {
				instant.store(instant.load(Ordering::SeqCst) + Duration::from_millis(42), Ordering::SeqCst);
			}));
		}
		for thread in threads {
			thread.join().unwrap();
		}

		assert!(instant.load(Ordering::Relaxed) >= Instant::now() - Duration::from_millis(42 * 4));
	}

	#[tokio::test]
	async fn task_start_soft() {
		let finished = Arc::new(AtomicBool::new(false));

		let mut slot = TaskSlot::new(Tokio);
		slot.insert(TaskStartMode::Soft, {
			let finished = Arc::clone(&finished);
			async move {
				finished.store(true, Ordering::Release);
			}
		});

		assert!(!slot.insert(TaskStartMode::Soft, async {}));

		yield_now().await;
		assert!(finished.load(Ordering::Acquire));
	}

	#[tokio::test]
	async fn task_start_override() {
		let finished = Arc::new(AtomicBool::new(false));

		let mut slot = TaskSlot::new(Tokio);
		slot.insert(TaskStartMode::Soft, {
			let finished = Arc::clone(&finished);
			async move {
				finished.store(true, Ordering::Release);
			}
		});

		assert!(slot.insert(TaskStartMode::Override, async {}));

		yield_now().await;
		assert!(finished.load(Ordering::Acquire));
	}

	#[tokio::test]
	async fn task_start_abort() {
		let finished = Arc::new(AtomicBool::new(false));

		let mut slot = TaskSlot::new(Tokio);
		slot.insert(TaskStartMode::Soft, {
			let finished = Arc::clone(&finished);
			async move {
				finished.store(true, Ordering::Release);
			}
		});

		assert!(slot.insert(TaskStartMode::Abort, async {}));

		yield_now().await;
		assert!(!finished.load(Ordering::Acquire));
	}
}
