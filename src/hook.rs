//! Provides the [`Hook`] trait and implementations of it for GUI libraries.
//!
//! `Hook` connects SWR to your GUI so it can trigger your application to rerender when data changes.
//!
//! SWR provides `Hook` implementations for the following GUI libraries:
//! - **[`egui`]** - [`Egui`]

#[cfg(feature = "egui")]
mod egui;
#[cfg(feature = "egui")]
#[cfg_attr(docsrs, doc(cfg(feature = "egui")))]
pub use self::egui::Egui;

/// `Hook`s connect SWR to the GUI engine, allowing SWR to request UI redraws when data changes, and detect when keys
/// are no longer used.
pub trait Hook: Send + Sync {
	/// Called when the application's viewport should be redrawn to display updated state.
	fn request_redraw(&self);

	/// Called to register a function to run at the end of each frame.
	///
	/// This function handles key lifecycles and is very important for proper operation!
	fn register_end_frame_cb(&self, cb: Box<dyn Fn() + Send + Sync>);

	/// Returns whether or not the application is currently focused.
	fn focused(&self) -> bool;

	/// Returns whether or not this render is the result of the window coming into focus.
	fn was_focus_triggered(&self) -> bool;
}

#[doc(hidden)]
mod mock {
	use std::sync::Arc;

	use parking_lot::Mutex;

	#[derive(Default)]
	pub struct MockHookInner {
		pub focus_triggered: bool,
		pub focused: bool,
		pub wants_redraw: bool,
		pub end_frame_cb: Option<Box<dyn Fn() + Send + Sync>>
	}

	#[derive(Default, Clone)]
	pub struct MockHook(pub Arc<Mutex<MockHookInner>>);

	impl MockHook {
		pub fn set_focus_triggered(&self, triggered: bool) {
			self.0.lock().focus_triggered = triggered;
		}

		pub fn set_focused(&self, focus: bool) {
			self.0.lock().focused = focus;
		}

		pub fn take_wants_redraw(&self) -> bool {
			std::mem::replace(&mut self.0.lock().wants_redraw, false)
		}

		pub fn within<R, F: FnOnce() -> R>(&self, f: F) -> R {
			let res = f();
			self.end_frame();
			res
		}

		pub fn end_frame(&self) {
			let inner = self.0.lock();
			if let Some(cb) = inner.end_frame_cb.as_ref() {
				cb();
			}
		}
	}

	impl super::Hook for MockHook {
		fn focused(&self) -> bool {
			self.0.lock().focused
		}

		fn was_focus_triggered(&self) -> bool {
			self.0.lock().focus_triggered
		}

		fn request_redraw(&self) {
			self.0.lock().wants_redraw = true;
		}

		fn register_end_frame_cb(&self, cb: Box<dyn Fn() + Send + Sync>) {
			self.0.lock().end_frame_cb = Some(cb);
		}
	}
}

#[doc(hidden)]
pub use self::mock::MockHook;
