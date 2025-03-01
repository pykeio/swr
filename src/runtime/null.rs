use std::{future::Future, marker::PhantomData, time::Duration};

/// A runtime that intentionally cannot be constructed. This forces users of SWR to specify their own runtime if either
/// zero or more than one runtime features are enabled.
#[derive(Clone)]
pub enum NullRuntime {}

impl super::Runtime for NullRuntime {
	type Task<T: Send + 'static> = NullHandle<T>;

	fn spawn<F>(&self, _future: F) -> Self::Task<F::Output>
	where
		F: Future + Send + 'static,
		F::Output: Send + 'static
	{
		unreachable!()
	}

	async fn wait(&self, _duration: Duration) {
		unreachable!()
	}
}

pub struct NullHandle<T>(PhantomData<T>);

unsafe impl<T: Send> Send for NullHandle<T> {}
unsafe impl<T: Send> Sync for NullHandle<T> {}

impl<T: Send + 'static> super::Task<T> for NullHandle<T> {
	fn abort(self) {
		unreachable!()
	}

	fn is_finished(&self) -> bool {
		unreachable!()
	}
}
