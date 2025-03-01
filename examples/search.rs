use std::time::Duration;

use egui::CentralPanel;
use serde::de::DeserializeOwned;
use tokio::{runtime::Runtime, time::sleep};

struct Fetcher;

impl swr::Fetcher for Fetcher {
	type Response<T: Send + Sync + 'static> = T;
	type Error = serde_json::Error;
	type Key = String;

	async fn fetch<T: DeserializeOwned + Send + Sync + 'static>(&self, key: &Self::Key) -> Result<Self::Response<T>, Self::Error> {
		sleep(Duration::from_secs(1)).await;

		serde_json::from_value(serde_json::json!(format!("results for {key}...")))
	}
}

fn main() -> eframe::Result<()> {
	tracing_subscriber::fmt()
		.with_env_filter(tracing_subscriber::EnvFilter::new("swr=debug"))
		.init();

	let rt = Runtime::new().expect("unable to create tokio runtime");
	let _enter = rt.enter();

	eframe::run_native(
		"SWR example",
		eframe::NativeOptions {
			centered: true,
			viewport: egui::ViewportBuilder::default().with_inner_size([300.0, 120.0]),
			..Default::default()
		},
		Box::new(|cc| {
			let app = Application::new(cc);
			Ok(Box::new(app))
		})
	)
}

struct Application {
	swr: swr::SWR<Fetcher, swr::runtime::Tokio>,
	query_slot: Option<swr::Persisted<String, Fetcher, swr::runtime::Tokio>>,
	search_query: String
}

impl Application {
	pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
		let swr = swr::new_in(Fetcher, swr::runtime::Tokio, swr::hook::Egui::new(&cc.egui_ctx));
		Self {
			swr,
			query_slot: None,
			search_query: String::new()
		}
	}
}

impl eframe::App for Application {
	fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
		CentralPanel::default().show(ctx, |ui| {
			ui.add_space(30.0);
			ui.vertical_centered(|ui| {
				if ui.add(egui::TextEdit::singleline(&mut self.search_query)).changed() {
					if !self.search_query.is_empty() {
						let old_data = self.query_slot.take().and_then(|c| c.get_shallow().data);
						self.query_slot = Some(self.swr.persisted(&self.search_query, swr::Options {
							fallback: old_data,
							..swr::Options::immutable()
						}));
					} else {
						self.query_slot.take();
					}
				}

				if let Some(slot) = self.query_slot.as_ref() {
					let result = slot.get();
					if result.loading {
						ui.spinner();
					} else {
						let data = result.data.as_deref().unwrap();
						ui.heading(data);
					}
				} else {
					ui.label("Start typing to search");
				}
			})
		});
	}
}
