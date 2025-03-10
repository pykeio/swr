use std::{
	future::Future,
	sync::{Arc, Weak, atomic::Ordering}
};

use serde::de::DeserializeOwned;

use crate::{
	SWRInner,
	cache::{CacheSlot, StateAccessor},
	error::Error,
	fetcher::Fetcher,
	options::{MutateOptions, Options},
	revalidate::{RevalidateIntent, launch_fetch},
	runtime::{DefaultRuntime, Runtime},
	util::TaskStartMode
};

/// A persisted slot in the [cache][crate::SWR].
///
/// The cache entry will not be garbage collected for as long as the slot is held.
pub struct Persisted<T: Send + Sync + 'static, F: Fetcher, R: Runtime = DefaultRuntime> {
	slot: CacheSlot<F>,
	options: Option<Options<T, F>>,
	inner: Arc<SWRInner<F, R>>
}

impl<T, F, R> Persisted<T, F, R>
where
	T: DeserializeOwned + Send + Sync + 'static,
	F: Fetcher,
	R: Runtime
{
	pub(crate) fn new(swr: &Arc<SWRInner<F, R>>, slot: CacheSlot<F>, options: Option<Options<T, F>>) -> Self {
		{
			let states = swr.cache.states();
			if let Some(state) = states.get(slot) {
				state.persisted_instances.fetch_add(1, Ordering::Relaxed);
				if let Some(options) = options.as_ref() {
					state.options.write().update_from(options);
				}
			}
		}

		Self {
			slot,
			options,
			inner: Arc::clone(swr)
		}
	}

	/// Triggers the cache entry to revalidate.
	///
	/// This function can be used outside of the GUI.
	pub fn revalidate(&self) {
		self.inner.revalidate(self.slot);
	}

	/// Returns this slot's entry in the cache.
	///
	/// This should only be used during the GUI's rendering process. For use outside of the GUI, see
	/// [`Persisted::get_shallow`].
	pub fn get(&self) -> FetchResult<T, F, R> {
		let states = self.inner.cache.states();
		self.get_inner(states, true)
	}

	/// Returns this slot's entry in the cache.
	///
	/// Unlike [`Persisted::get`], this does not contribute to the lifecycle of the cache entry, thus it is suitable for
	/// use outside of the GUI.
	pub fn get_shallow(&self) -> FetchResult<T, F, R> {
		let states = self.inner.cache.states();
		self.get_inner(states, false)
	}

	fn get_inner(&self, mut states: StateAccessor<'_, F, R>, update: bool) -> FetchResult<T, F, R> {
		let Some(state) = states.get(self.slot) else {
			return FetchResult::new_empty(self.slot, Arc::downgrade(&self.inner));
		};
		let was_alive = state.alive.load(Ordering::Acquire);

		let mut error = state.error.as_ref().map(|e| Error::Fetcher(Arc::clone(e)));
		let data = match state.data::<T>() {
			Some(Ok(data)) => Some(data),
			Some(Err(e)) => {
				error = error.or(Some(Error::MismatchedType(e)));
				self.options.as_ref().and_then(|o| o.fallback.clone())
			}
			None => self.options.as_ref().and_then(|o| o.fallback.clone())
		};
		let (mut loading, mut validating) = (state.loading, state.validating);

		if update {
			let options = state.options.read();

			if self.inner.hook.was_focus_triggered()
				&& options.revalidate_on_focus
				&& state.last_draw_time.load(Ordering::Acquire).elapsed() >= options.focus_throttle_interval
			{
				state.revalidate_intent.add(RevalidateIntent::APPLICATION_FOCUSED);
			}

			if !was_alive {
				if (options.fetch_on_first_use && data.is_none())
					// fetch task aborted before it could finish. instead of having the key be forever stuck in the
					// loading state, restart the initial fetch
					|| (state.loading && state.fetch_task.is_finished())
				{
					state.revalidate_intent.add(RevalidateIntent::FIRST_USAGE);
				} else {
					// TODO: configurable
					state.revalidate_intent.add(RevalidateIntent::STALE);
				}
			}

			state.mark_used();

			let intent = state.revalidate_intent.take();
			if intent != 0 {
				drop((state, options));
				states.mutate(self.slot, |state| {
					launch_fetch::<T, F, R>(state, &self.inner, self.slot, TaskStartMode::Soft, intent);
					(loading, validating) = (state.loading, state.validating);
				});
			}
		}

		FetchResult {
			data,
			error,
			loading,
			validating,
			slot: self.slot,
			inner: Arc::downgrade(&self.inner)
		}
	}

	/// Replaces the slot's entry in the cache with a successful result containing `data`.
	///
	/// This function can be used outside of the GUI.
	pub fn mutate(&self, data: Arc<F::Response<T>>)
	where
		T: Send + Sync + 'static
	{
		self.inner.mutate(self.slot, data);
	}

	/// Asynchronously mutates this slot's cache entry.
	///
	/// The `mutator` is given the entry's current data (if present) and a reference to the cache's [`Fetcher`], and
	/// returns a fallible future whose result will populate the cache. This value is also returned via a [runtime
	/// `Task`][`runtime::Task`] which may be awaited on (depending on the exact choice of [`Runtime`]).
	///
	/// [`MutateOptions`] also allows for more control over how the mutation occurs.
	pub fn mutate_with<U, M, E, Fut>(&self, options: MutateOptions<F, T, U>, mutator: M) -> R::Task<Result<U, E>>
	where
		U: Send,
		M: FnOnce(Option<Arc<F::Response<T>>>, &F) -> Fut + Send + 'static,
		E: Send,
		Fut: Future<Output = std::result::Result<U, E>> + Send
	{
		let data = self.get_shallow().data;
		self.inner.mutate_with(self.slot, data, options, mutator)
	}
}

impl<T: Send + Sync + 'static, F: Fetcher, R: Runtime> Drop for Persisted<T, F, R> {
	fn drop(&mut self) {
		let states = self.inner.cache.states();
		let Some(state) = states.get(self.slot) else {
			return;
		};
		state.persisted_instances.fetch_sub(1, Ordering::Release);
	}
}

#[derive(Clone)]
pub struct FetchResult<T: Send + Sync + 'static, F: Fetcher, R: Runtime = DefaultRuntime> {
	pub data: Option<Arc<F::Response<T>>>,
	pub error: Option<Error<F>>,
	pub loading: bool,
	pub validating: bool,
	pub(crate) slot: CacheSlot<F>,
	pub(crate) inner: Weak<SWRInner<F, R>>
}

impl<T: Send + Sync + 'static, F: Fetcher, R: Runtime> FetchResult<T, F, R> {
	pub(crate) fn new_empty(slot: CacheSlot<F>, inner: Weak<SWRInner<F, R>>) -> Self {
		FetchResult {
			data: None,
			error: None,
			loading: false,
			validating: false,
			slot,
			inner
		}
	}

	/// Triggers the cache entry to revalidate.
	///
	/// This function can be used outside of the GUI.
	pub fn revalidate(&self) {
		let Some(inner) = self.inner.upgrade() else {
			return;
		};
		inner.revalidate(self.slot);
	}

	/// Replaces the entry in the cache with a successful result containing `data`.
	///
	/// This function can be used outside of the GUI.
	pub fn mutate(&self, data: Arc<F::Response<T>>)
	where
		T: Send + Sync + 'static
	{
		let Some(inner) = self.inner.upgrade() else {
			return;
		};
		inner.mutate(self.slot, data);
	}

	/// Asynchronously mutates this cache entry.
	///
	/// The `mutator` is given the entry's current data (if present) and a reference to the cache's [`Fetcher`], and
	/// returns a fallible future whose result will populate the cache. This value is also returned via a [runtime
	/// `Task`][`runtime::Task`] which may be awaited on (depending on the exact choice of [`Runtime`]).
	///
	/// [`MutateOptions`] also allows for more control over how the mutation occurs.
	pub fn mutate_with<U, M, E, Fut>(&self, options: MutateOptions<F, T, U>, mutator: M) -> Option<R::Task<Result<U, E>>>
	where
		U: Send,
		M: FnOnce(Option<Arc<F::Response<T>>>, &F) -> Fut + Send + 'static,
		E: Send,
		Fut: Future<Output = std::result::Result<U, E>> + Send
	{
		let inner = self.inner.upgrade()?;
		Some(inner.mutate_with(self.slot, self.data.clone(), options, mutator))
	}
}
