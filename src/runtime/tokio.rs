use std::{future::Future, time::Duration};

use tokio::{runtime::Handle, task::JoinHandle};

/// An asynchronous runtime using [`tokio`] via the global runtime context.
#[derive(Clone, Default)]
#[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
pub struct Tokio;

impl super::Runtime for Tokio {
	type Task<T: Send + 'static> = JoinHandle<T>;

	fn spawn<F>(&self, future: F) -> Self::Task<F::Output>
	where
		F: Future + Send + 'static,
		F::Output: Send + 'static
	{
		tokio::spawn(future)
	}

	fn wait(&self, duration: Duration) -> impl Future<Output = ()> {
		tokio::time::sleep(duration)
	}
}

/// An asynchronous runtime using [`tokio`] via a runtime [`Handle`].
#[derive(Clone)]
#[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
pub struct TokioHandle(Handle);

impl TokioHandle {
	/// Creates a runtime using the `tokio` runtime [`Handle`].
	pub fn new(handle: Handle) -> Self {
		TokioHandle(handle)
	}
}

impl super::Runtime for TokioHandle {
	type Task<T: Send + 'static> = JoinHandle<T>;

	fn spawn<F>(&self, future: F) -> Self::Task<F::Output>
	where
		F: Future + Send + 'static,
		F::Output: Send + 'static
	{
		self.0.spawn(future)
	}

	fn wait(&self, duration: Duration) -> impl Future<Output = ()> {
		let _guard = self.0.enter();
		tokio::time::sleep(duration)
	}
}

impl<T: Send + 'static> super::Task<T> for JoinHandle<T> {
	fn abort(self) {
		JoinHandle::<T>::abort(&self);
	}

	fn is_finished(&self) -> bool {
		JoinHandle::<T>::is_finished(self)
	}
}
