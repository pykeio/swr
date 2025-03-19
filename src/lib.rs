//! Data fetching library for immediate-mode GUIs.
//!
//! SWR operates on the "stale-while-revalidate" principle: *stale* data is shown while it is *revalidated* in the
//! background. This revalidation can be configured to occur when the application is focused, or at a set interval to
//! ensure data is always up to date.
//!
//! To create an SWR cache, you need a **[`Fetcher`]**, a **[`Hook`]**, and optionally a **[`Runtime`]**. The `Fetcher`
//! is responsible for retrieving the data (e.g. from a remote server) when it needs to be revalidated. The `Hook`
//! connects to your GUI so SWR can trigger your application to rerender when data changes. The `Runtime` connects SWR
//! to an async runtime like [`tokio`] so that it can spawn fetch tasks in the background.
//!
//! Retrieving data from the SWR cache is done with a **key**. This key can be anything from a simple string
//! representing a URL path like `/todos/{uuid}`, or it can be a complex user-defined type. Each key's data and state is
//! shared across all usages of the key.
//!
//! Along with manually triggering revalidations, keys can also be **mutated** to either immediately override the data
//! stored in the cache, or modify it based on the result of an async task.
//!
//! # Hooks
//! SWR provides [`Hook`] implementations for the following GUI libraries:
//! - **[`egui`]** - [`hook::Egui`] (available with the `egui` Cargo feature)
//! - *write your own by implementing [`Hook`]!*
//!
//! # Runtimes
//! SWR provides [`Runtime`] implementations for the following async runtimes:
//! - **[`tokio`]** - [`runtime::Tokio`]/[`runtime::TokioHandle`] (available with the `tokio` Cargo feature **and
//!   enabled by default**)
//! - **[`smol`]** - [`runtime::Smol`] (available with the `smol` Cargo feature)
//! - *write your own by implementing [`Runtime`]!*
//!
//! [`swr::new`][crate::new] creates a new SWR cache using the *default runtime*. With SWR's default Cargo features,
//! this is the `tokio` runtime, and you must set up your application to create a `tokio` runtime before using
//! SWR. If you disable default features and enable exactly one other runtime feature (like `smol`), then that will be
//! the default runtime instead.
//!
//! If you enable multiple runtime features (i.e. you added `smol` without disabling default features), or do not enable
//! any runtime features (`default-features = false`), then you must manually specify the runtime using
//! [`swr::new_in`][crate::new_in] instead.
//!
//! # Other Cargo features
//! - **`tracing`**: Enables logging when fetches occur/cache entries are garbage collected, via [`tracing`].

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(rust_2024_compatibility)]
#![allow(clippy::tabs_in_doc_comments)]
#![warn(missing_docs)]

use std::{
	borrow::Borrow,
	future::Future,
	hash::Hash,
	sync::{Arc, atomic::Ordering}
};

use serde::de::DeserializeOwned;

pub(crate) mod cache;
pub(crate) mod error;
pub(crate) mod fetcher;
pub mod hook;
pub(crate) mod options;
pub(crate) mod result;
pub(crate) mod revalidate;
pub mod runtime;
pub(crate) mod util;

use self::{
	cache::{Cache, CacheEntryStatus, CacheSlot},
	revalidate::RevalidateIntent,
	runtime::{DefaultRuntime, RuntimeDefault}
};
pub use self::{
	error::{Error, MismatchedTypeError},
	fetcher::Fetcher,
	hook::Hook,
	options::{MutateOptions, Options},
	result::{FetchResult as Result, Persisted},
	runtime::Runtime
};

pub(crate) struct SWRInner<F: Fetcher, R: Runtime> {
	fetcher: F,
	runtime: R,
	hook: Box<dyn Hook>,
	cache: Cache<F, R>
}

impl<F: Fetcher, R: Runtime> SWRInner<F, R> {
	pub(crate) fn new<H: Hook + 'static>(fetcher: F, runtime: R, hook: H) -> Self {
		Self {
			fetcher,
			runtime: runtime.clone(),
			hook: Box::new(hook) as Box<dyn Hook>,
			cache: Cache::new(runtime)
		}
	}

	pub(crate) fn revalidate(&self, slot: CacheSlot) {
		let states = self.cache.states();
		let Some(state) = states.get(slot) else {
			return;
		};
		state.revalidate_intent().add(RevalidateIntent::MANUALLY_TRIGGERED);
		self.hook.request_redraw();
	}

	pub(crate) fn mutate<T>(&self, slot: CacheSlot, data: Arc<F::Response<T>>)
	where
		T: Send + Sync + 'static
	{
		let mut states = self.cache.states();
		states.mutate(slot, |state| {
			state.insert(data);
			self.hook.request_redraw();
		});
	}

	pub(crate) fn mutate_with<T, U, M, E, Fut>(
		self: &Arc<Self>,
		slot: CacheSlot,
		data: Option<Arc<F::Response<T>>>,
		options: MutateOptions<F::Response<T>, U>,
		mutator: M
	) -> R::Task<std::result::Result<U, E>>
	where
		T: Send + Sync + 'static,
		U: Send,
		M: FnOnce(Option<Arc<F::Response<T>>>, &F) -> Fut + Send + 'static,
		E: Send,
		Fut: Future<Output = std::result::Result<U, E>> + Send
	{
		let inner = Arc::clone(self);
		self.runtime.spawn(async move {
			let previous_data = if let Some(optimistic_data) = options.optimistic_data {
				let mut states = inner.cache.states();
				states
					.mutate(slot, |state| {
						let old_data = state.insert(optimistic_data);
						inner.hook.request_redraw();
						old_data
					})
					.flatten()
			} else {
				None
			};

			let res = mutator(data, &inner.fetcher).await;

			{
				let mut states = inner.cache.states();
				states.mutate(slot, |state| {
					// If we're currently in the middle of a fetch, cancel it since it's probably outdated.
					state.fetch_task.abort();

					if let Ok(data) = &res {
						state.insert((options.populator)(data));
						if options.revalidate {
							state.revalidate_intent().add(RevalidateIntent::MUTATE);
						}
					} else if options.rollback_on_error {
						if let Some(previous_data) = previous_data {
							state.insert_untyped(
								previous_data.value,
								#[cfg(debug_assertions)]
								previous_data.type_name
							);
						}
					}

					inner.hook.request_redraw();
				});
			}

			res
		})
	}
}

/// An SWR cache.
///
/// See [the crate-level documentation][crate] for more information.
///
/// # Cloning
/// `SWR` is internally reference counted via [`Arc`], so it can be cheaply cloned.
#[derive(Clone)]
pub struct SWR<F: Fetcher, R: Runtime = DefaultRuntime> {
	inner: Arc<SWRInner<F, R>>
}

impl<F: Fetcher, R: Runtime> SWR<F, R> {
	/// Creates a new SWR cache.
	///
	/// To use this constructor, the [`Runtime`] (`R`) must implement [`Default`], which is the case if using SWR's
	/// [default runtime][crate#runtimes] (i.e. not specifying `R`).
	#[inline]
	pub fn new<H: Hook + 'static>(fetcher: F, hook: H) -> Self
	where
		R: RuntimeDefault
	{
		Self::new_in(fetcher, R::default(), hook)
	}

	/// Creates a new SWR cache using a non-default [`Runtime`].
	pub fn new_in<H: Hook + 'static>(fetcher: F, runtime: R, hook: H) -> Self {
		let inner = Arc::new(SWRInner::new(fetcher, runtime, hook));

		{
			let weak_inner = Arc::downgrade(&inner);
			inner.hook.register_end_frame_cb(Box::new(move || {
				if let Some(inner) = weak_inner.upgrade() {
					inner.cache.retain(|_, state| {
						let status = state.status();
						let used = status.clear(CacheEntryStatus::USED_THIS_PASS, Ordering::AcqRel);
						if !used {
							let was_alive = status.clear(CacheEntryStatus::ALIVE, Ordering::AcqRel);
							if !was_alive && state.strong_count.load(Ordering::Acquire) == 0 {
								let should_gc = match state.options.read().garbage_collect_timeout() {
									Some(timeout) => state.last_draw_time(Ordering::Relaxed).elapsed() >= timeout,
									None => false
								};
								if should_gc {
									#[cfg(feature = "tracing")]
									{
										tracing::info!(key = ?state.key(), "clearing entry because it exceeded GC timeout");
									}

									state.fetch_task.abort();
									state.refresh_task.abort();
									state.retry_task.abort();

									return false;
								}
							}
						} else {
							status.set(CacheEntryStatus::ALIVE, Ordering::Release);
						}
						true
					})
				}
			}));
		}

		Self { inner }
	}

	/// Returns a [persisted cache slot][Persisted] for the given key.
	///
	/// Persisted slots are meant to be stored across renders; they are thus more performant than the more
	/// immediate-style [`SWR::get`] functions.
	///
	/// The cache entry's `options` will be [merged][Options#merging-behavior] if the key already exists in the cache.
	pub fn persisted<T, K>(&self, key: &K, options: Options<F::Response<T>>) -> Persisted<T, F, R>
	where
		T: DeserializeOwned + Send + Sync + 'static,
		K: Hash + Eq + ?Sized,
		F::Key: Borrow<K> + for<'k> From<&'k K>
	{
		Persisted::<T, F, R>::new(&self.inner, self.inner.cache.get_or_create(key), Some(options))
	}

	/// Returns the key's entry in the cache, using the default [options][Options].
	///
	/// This should only be used during the GUI's rendering process. For use outside of the GUI, see
	/// [`SWR::get_shallow`].
	///
	/// # Performance
	/// This function is equivalent to creating a persisted entry and immediately discarding it on each render and thus
	/// performs more computation than necessary. If performance is a concern, you should use [`SWR::persisted`]
	/// instead.
	pub fn get<T, K>(&self, key: &K) -> Result<T, F, R>
	where
		T: DeserializeOwned + Send + Sync + 'static,
		K: Hash + Eq + ?Sized,
		F::Key: Borrow<K> + for<'k> From<&'k K>
	{
		Persisted::<T, F, R>::new(&self.inner, self.inner.cache.get_or_create(key), None).get()
	}

	/// Returns the key's entry in the cache.
	///
	/// The cache entry's `options` will be [merged][Options#merging-behavior] if the key already exists in the cache.
	///
	/// This should only be used during the GUI's rendering process. For use outside of the GUI, see
	/// [`SWR::get_shallow`].
	///
	/// # Performance
	/// This function is equivalent to creating a persisted entry and immediately discarding it on each render and thus
	/// performs more computation than necessary. If performance is a concern, you should use [`SWR::persisted`]
	/// instead.
	pub fn get_with<T, K>(&self, key: &K, options: Options<F::Response<T>>) -> Result<T, F, R>
	where
		T: DeserializeOwned + Send + Sync + 'static,
		K: Hash + Eq + ?Sized,
		F::Key: Borrow<K> + for<'k> From<&'k K>
	{
		Persisted::<T, F, R>::new(&self.inner, self.inner.cache.get_or_create(key), Some(options)).get()
	}

	/// Returns this key's entry in the cache, or `None` if it does not exist.
	///
	/// Unlike [`SWR::get`], this does not create the key if it does not exist, or contribute to the lifecycle of the
	/// cache entry; thus it is suitable for use outside of the GUI.
	pub fn get_shallow<T, K>(&self, key: &K) -> Option<Result<T, F, R>>
	where
		T: DeserializeOwned + Send + Sync + 'static,
		K: Hash + Eq + ?Sized,
		F::Key: Borrow<K> + for<'k> From<&'k K>
	{
		self.inner
			.cache
			.get(key)
			.map(|slot| Persisted::<T, F, R>::new(&self.inner, slot, None).get_shallow())
	}

	/// Triggers the key to revalidate, if it exists in the cache.
	///
	/// This function can be used outside of the GUI.
	pub fn revalidate<K>(&self, key: &K)
	where
		K: Hash + Eq + ?Sized,
		F::Key: Borrow<K> + for<'k> From<&'k K>
	{
		if let Some(slot) = self.inner.cache.get(key) {
			self.inner.revalidate(slot);
		}
	}

	/// Replaces the key's entry in the cache with a successful result containing `data`, creating the entry if it
	/// doesn't exist.
	///
	/// This function can be used outside of the GUI.
	pub fn mutate<T, K>(&self, key: &K, data: Arc<F::Response<T>>)
	where
		T: Send + Sync + 'static,
		K: Hash + Eq + ?Sized,
		F::Key: Borrow<K> + for<'k> From<&'k K>
	{
		self.inner.mutate(self.inner.cache.get_or_create(key), data);
	}

	/// Asynchronously mutates the cache entry with the given `key`, creating it if it doesn't exist.
	///
	/// The `mutator` is given the entry's current data (if present) and a reference to this cache's [`Fetcher`], and
	/// returns a fallible future whose result will populate the cache. This value is also returned via a [runtime
	/// `Task`][`runtime::Task`] which may be awaited on (depending on the exact choice of [`Runtime`]).
	///
	/// [`MutateOptions`] also allows for more control over how the mutation occurs.
	pub fn mutate_with<T, U, K, M, E, Fut>(&self, key: &K, options: MutateOptions<F::Response<T>, U>, mutator: M) -> R::Task<std::result::Result<U, E>>
	where
		T: Send + Sync + 'static,
		U: Send,
		K: Hash + Eq + ?Sized,
		F::Key: Borrow<K> + for<'k> From<&'k K>,
		M: FnOnce(Option<Arc<F::Response<T>>>, &F) -> Fut + Send + 'static,
		E: Send,
		Fut: Future<Output = std::result::Result<U, E>> + Send
	{
		let slot = self.inner.cache.get_or_create(key);
		let existing_data = self
			.inner
			.cache
			.states()
			.get(slot)
			.and_then(|entry| entry.data::<T>().and_then(std::result::Result::ok));
		self.inner.mutate_with(slot, existing_data, options, mutator)
	}
}

/// Creates a new SWR cache.
///
/// To use this constructor, the [`Runtime`] (`R`) must implement [`Default`], which is the case if using SWR's
/// [default runtime][crate#runtimes] (i.e. not specifying `R`).
#[inline(always)]
pub fn new<F: Fetcher, R: Runtime + RuntimeDefault, H: Hook + 'static>(fetcher: F, hook: H) -> SWR<F, R> {
	SWR::new(fetcher, hook)
}

/// Creates a new SWR cache using a non-default [`Runtime`].
#[inline(always)]
pub fn new_in<F: Fetcher, R: Runtime, H: Hook + 'static>(fetcher: F, runtime: R, hook: H) -> SWR<F, R> {
	SWR::new_in(fetcher, runtime, hook)
}
