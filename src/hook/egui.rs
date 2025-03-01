use std::sync::Arc;

use egui::{Context, Event};

/// A [`Hook`](super::Hook) for [`egui`] applications.
///
/// Using `EguiHook` requires that you pass in the `egui::Context`. With [`eframe`](https://crates.io/crates/eframe), this might look like:
/// ```no_run
/// # fn wrapper() {
/// fn main() -> eframe::Result<()> {
/// 	// ...initialize async runtime...
///
/// 	eframe::run_native(
/// 		"SWR example",
/// 		eframe::NativeOptions::default(),
/// 		Box::new(|cc| {
/// 			let hook = swr::EguiHook::new(&cc.egui_ctx);
/// 			# let _ = stringify! {
/// 			let swr = swr::new(Fetcher, hook);
/// 			# };
/// 			# let swr = 42;
///
/// 			Ok(Box::new(Application { swr }))
/// 		})
/// 	)
/// }
///
/// # let _ = stringify! {
/// struct Application {
/// 	swr: swr::SWR<Fetcher>
/// }
/// # };
/// # struct Application { swr: u32 }
///
/// impl eframe::App for Application {
/// 	fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
/// 		# let _ = stringify! {
/// 		...
/// 		# };
/// 		# unimplemented!();
/// 	}
/// }
/// # }
/// ```
pub struct Egui {
	context: Context
}

impl Egui {
	/// Creates a new `egui` hook using the given context.
	pub fn new(context: &Context) -> Self {
		Self { context: context.clone() }
	}
}

impl super::Hook for Egui {
	fn request_redraw(&self) {
		self.context.request_repaint();
	}

	fn was_focus_triggered(&self) -> bool {
		self.context.input(|i| i.events.iter().any(|e| matches!(e, Event::WindowFocused(true))))
	}

	fn focused(&self) -> bool {
		self.context.input(|i| i.focused)
	}

	fn register_end_frame_cb(&self, cb: Box<dyn Fn() + Send + Sync>) {
		self.context.on_end_pass("swr", Arc::new(move |_| cb()));
	}
}
