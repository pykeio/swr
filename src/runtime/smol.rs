use std::{future::Future, time::Duration};

use smol::Task;

/// An asynchronous runtime using [`smol`].
#[derive(Clone, Default)]
#[cfg_attr(docsrs, doc(cfg(feature = "smol")))]
pub struct Smol;

impl super::Runtime for Smol {
	type Task<T: Send + 'static> = Task<T>;

	fn spawn<F>(&self, future: F) -> Self::Task<F::Output>
	where
		F: Future + Send + 'static,
		F::Output: Send + 'static
	{
		smol::spawn(future)
	}

	async fn wait(&self, duration: Duration) {
		smol::Timer::after(duration).await;
	}
}

impl<T: Send + 'static> super::Task<T> for Task<T> {
	fn abort(self) {
		drop(Task::<T>::cancel(self));
	}

	fn is_finished(&self) -> bool {
		Task::<T>::is_finished(self)
	}
}
