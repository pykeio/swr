use std::time::Duration;

use egui::CentralPanel;
use serde::de::DeserializeOwned;
use smol::Timer;

struct Fetcher;

impl swr::Fetcher for Fetcher {
	type Response<T: Send + Sync + 'static> = T;
	type Error = serde_json::Error;
	type Key = String;

	async fn fetch<T: DeserializeOwned + Send + Sync + 'static>(&self, key: &Self::Key) -> Result<Self::Response<T>, Self::Error> {
		Timer::after(Duration::from_secs(2)).await;

		match key.as_str() {
			"/the-answer" => serde_json::from_str("42"),
			_ => panic!()
		}
	}
}

fn main() -> eframe::Result<()> {
	tracing_subscriber::fmt()
		.with_env_filter(tracing_subscriber::EnvFilter::new("swr=debug"))
		.init();

	eframe::run_native(
		"SWR example",
		eframe::NativeOptions {
			centered: true,
			viewport: egui::ViewportBuilder::default().with_inner_size([300.0, 120.0]),
			..Default::default()
		},
		Box::new(|cc| {
			let swr = swr::new_in(Fetcher, swr::runtime::Smol, swr::hook::Egui::new(&cc.egui_ctx));

			let app = Application { swr };
			Ok(Box::new(app))
		})
	)
}

struct Application {
	swr: swr::SWR<Fetcher, swr::runtime::Smol>
}

impl eframe::App for Application {
	fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
		CentralPanel::default().show(ctx, |ui| {
			ui.add_space(30.0);
			ui.vertical_centered(|ui| {
				let result = self.swr.get::<u32, _>("/the-answer");

				if result.loading {
					ui.spinner();
				} else {
					let answer = result.data.as_deref().unwrap();
					ui.heading(format!("The answer is {answer}"));
				}

				if ui.add_enabled_ui(!result.validating, |ui| ui.button("Revalidate").clicked()).inner {
					result.revalidate();
				}
			})
		});
	}
}
