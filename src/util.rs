use std::{future::Future, sync::atomic::Ordering, time::Duration};

// Use `tokio`'s `Instant` wrapper in testing since we can 'advance' time with `tokio::time::advance`
#[cfg(test)]
pub type Instant = tokio::time::Instant;
#[cfg(not(test))]
pub type Instant = std::time::Instant;

use crate::runtime::{Runtime, Task};

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
pub fn throttle(prev_time: Option<Instant>, throttle_time: Option<Duration>) -> bool {
	match (prev_time, throttle_time) {
		(Some(prev_time), Some(throttle_time)) => prev_time.elapsed() >= throttle_time,
		_ => true
	}
}

pub(crate) trait AtomicBitwise {
	type Base: Copy;

	fn bits_get(&self, bits: Self::Base, ordering: Ordering) -> bool;
	fn bits_set(&self, bits: Self::Base, ordering: Ordering) -> bool;
	fn bits_clear(&self, bits: Self::Base, ordering: Ordering) -> bool;
}

macro_rules! impl_atomic_bitwise {
	($($atomic_ty:ty => $base_ty:ty),*) => {
		$(
			impl AtomicBitwise for $atomic_ty {
				type Base = $base_ty;

				fn bits_get(&self, bits: Self::Base, ordering: Ordering) -> bool {
					(self.load(ordering) & bits) != 0
				}
				fn bits_set(&self, bits: Self::Base, ordering: Ordering) -> bool {
					(self.fetch_or(bits, ordering) & bits) != 0
				}
				fn bits_clear(&self, bits: Self::Base, ordering: Ordering) -> bool {
					(self.fetch_and(!bits, ordering) & bits) != 0
				}
			}
		)*
	}
}

impl_atomic_bitwise! {
	std::sync::atomic::AtomicU8 => u8
}

#[cfg(test)]
mod tests {
	use std::sync::{
		Arc,
		atomic::{AtomicBool, Ordering}
	};

	use tokio::task::yield_now;

	use super::{TaskSlot, TaskStartMode};
	use crate::runtime::Tokio;

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
