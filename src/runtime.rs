//! Provides the [`Runtime`] trait and implementations of it for various async runtimes.
//!
//! `Runtime` connects SWR to an async runtime like [`tokio`] so that it can spawn fetch tasks in the background.
//!
//! SWR provides `Runtime` implementations for the following async runtimes:
//! - **[`tokio`]** - [`Tokio`]/[`TokioHandle`]
//! - **[`smol`]** - [`Smol`]

use std::{future::Future, time::Duration};

mod null;
#[cfg(feature = "smol")]
mod smol;
#[cfg(feature = "smol")]
#[cfg_attr(docsrs, doc(cfg(feature = "smol")))]
pub use self::smol::Smol;
#[cfg(feature = "tokio")]
mod tokio;
#[cfg(feature = "tokio")]
#[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
pub use self::tokio::{Tokio, TokioHandle};

cfg_if::cfg_if! {
	if #[cfg(all(feature = "tokio", not(feature = "smol")))] {
		#[doc(hidden)]
		pub type DefaultRuntime = self::tokio::Tokio;
	} else if #[cfg(all(feature = "smol", not(feature = "tokio")))] {
		#[doc(hidden)]
		pub type DefaultRuntime = self::smol::Smol;
	} else {
		#[doc(hidden)]
		pub type DefaultRuntime = self::null::NullRuntime;
	}
}

/// An asynchronous runtime, used to spawn fetch tasks.
///
/// SWR natively supports two async runtimes:
/// - [`tokio`][::tokio], via [`Tokio`] and [`TokioHandle`] (available with the `tokio` feature and enabled by default)
/// - [`smol`][::smol], via [`Smol`] (available with the `smol` feature)
///
/// If exactly one of the runtime Cargo features are enabled, you can use functions like [`swr::new`](`crate::new`) to
/// create an SWR cache for the default runtime.
///
/// If multiple runtime features are enabled, or you would like to use your own runtime, you must use
/// [`swr::new_in`](`crate::new_in`) to manually specify the runtime.
pub trait Runtime: Clone + Send + Sync + 'static {
	/// A handle to an asynchronous task spawned by [`Runtime::spawn`].
	type Task<T: Send + 'static>: Task<T>;

	/// Spawns a new asynchronous background task, returning a [handle][`Runtime::Task`] to it.
	fn spawn<F>(&self, future: F) -> Self::Task<F::Output>
	where
		F: Future + Send + 'static,
		F::Output: Send + 'static;

	/// Returns a future that, when awaited, causes the task to sleep for the specified `duration`; an asynchronous
	/// version of [`std::thread::sleep`].
	fn wait(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

/// Trait automatically implemented for `Runtime`s that also impl `Default` with improved diagnostics that warn about
/// runtime Cargo features.
#[diagnostic::on_unimplemented(
	message = "`{Self}` cannot be used automatically because it does not impl `Default`",
	note = "you may need to create the runtime with `{Self}::new` and pass it to `swr::new_in` instead",
	note = "if this is `NullRuntime`, that means you need to provide your own runtime or enable *exactly one* of `swr`'s runtime features, like `tokio` or `smol`"
)]
#[doc(hidden)]
pub trait RuntimeDefault: Runtime + Default {}
impl<T: Runtime + Default> RuntimeDefault for T {}

/// A handle to an asynchronous task spawned by a [`Runtime`].
///
/// SWR needs to be able to abort requests when an external mutation occurs, or when a key falls out of use while a
/// fetch is in progress.
///
/// Execution of the task should continue even if this handle is dropped.
pub trait Task<T>: Send + Sync + 'static {
	/// Flag this task for cancellation.
	fn abort(self);

	/// Returns `true` if the task is no longer running, either due to normal completion or abortion via
	/// [`Task::abort`].
	fn is_finished(&self) -> bool;
}
