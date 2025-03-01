use std::{
	borrow::{Borrow, Cow},
	hash::{Hash, Hasher}
};

use eframe::NativeOptions;
use egui::CentralPanel;
use serde::de::DeserializeOwned;
use tokio::runtime::Runtime;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OwnedKey {
	path: String,
	query_params: Vec<(String, String)>
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct BorrowedKey<'a> {
	path: &'a str,
	// if you're comfortable adding an extra dependency, using a `smallvec` might be useful here to reduce allocation
	// https://crates.io/crates/smallvec
	query_params: Vec<(&'a str, Cow<'a, str>)>
}

impl<'a> BorrowedKey<'a> {
	pub fn new(path: &'a str) -> Self {
		Self { path, query_params: Vec::default() }
	}

	pub fn with_query(mut self, name: &'a str, value: impl Into<Cow<'a, str>>) -> BorrowedKey<'a> {
		self.query_params.push((name, value.into()));
		self
	}
}

trait AsKey {
	fn as_key(&self) -> BorrowedKey<'_>;
}

impl AsKey for OwnedKey {
	fn as_key(&self) -> BorrowedKey<'_> {
		BorrowedKey {
			path: &self.path,
			query_params: self.query_params.iter().map(|(a, b)| (a.as_str(), Cow::Borrowed(&**b))).collect()
		}
	}
}

impl AsKey for BorrowedKey<'_> {
	fn as_key(&self) -> BorrowedKey<'_> {
		self.clone()
	}
}

impl From<&(dyn AsKey + '_)> for OwnedKey {
	fn from(key: &(dyn AsKey + '_)) -> Self {
		let key = key.as_key();
		OwnedKey {
			path: key.path.to_string(),
			query_params: key.query_params.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
		}
	}
}

impl<'a> Borrow<dyn AsKey + 'a> for OwnedKey {
	fn borrow(&self) -> &(dyn AsKey + 'a) {
		self
	}
}

impl PartialEq for dyn AsKey {
	fn eq(&self, other: &Self) -> bool {
		self.as_key().eq(&other.as_key())
	}
}

impl Eq for dyn AsKey {}

impl Hash for dyn AsKey {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.as_key().hash(state);
	}
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Comment {
	name: String,
	email: String,
	body: String
}

struct Fetcher;

impl swr::Fetcher for Fetcher {
	type Response<T: Send + Sync + 'static> = T;
	type Error = reqwest::Error;
	type Key = OwnedKey;

	async fn fetch<T: DeserializeOwned + Send + Sync + 'static>(&self, key: &Self::Key) -> Result<Self::Response<T>, Self::Error> {
		let mut url = String::from("https://jsonplaceholder.typicode.com");
		url.push_str(&key.path);
		if !key.query_params.is_empty() {
			url.push('?');
			url.push_str(&key.query_params.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"));
		}
		reqwest::get(url).await?.json().await
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
		NativeOptions::default(),
		Box::new(|cc| {
			let app = Application::new(cc);
			Ok(Box::new(app))
		})
	)
}

struct Application {
	post_id: usize,
	swr: swr::SWR<Fetcher, swr::runtime::Tokio>
}

impl Application {
	pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
		let swr = swr::new_in(Fetcher, swr::runtime::Tokio, swr::hook::Egui::new(&cc.egui_ctx));
		Self { post_id: 1, swr }
	}
}

impl eframe::App for Application {
	fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
		CentralPanel::default().show(ctx, |ui| {
			ui.vertical_centered(|ui| {
				ui.horizontal(|ui| {
					ui.heading(format!("Post #{}", self.post_id));
					if ui.button("+").clicked() {
						self.post_id = (self.post_id + 1).min(100);
					}
					if ui.button("-").clicked() {
						self.post_id = (self.post_id - 1).max(1);
					}
				});

				ui.separator();

				let comments = self
					.swr
					.get_with::<Vec<Comment>, _>(&BorrowedKey::new("/comments").with_query("postId", self.post_id.to_string()), swr::Options::immutable());
				egui::ScrollArea::vertical().show(ui, |ui| {
					if comments.loading {
						ui.spinner();
					} else {
						if let Some(error) = comments.error {
							ui.label(egui::RichText::new(error.to_string()).color(egui::Color32::DARK_RED));
							return;
						}

						for comment in comments.data.unwrap().as_ref() {
							egui::Frame::group(ui.style()).show(ui, |ui| {
								ui.vertical(|ui| {
									ui.label(egui::RichText::new(&comment.email).color(egui::Color32::DARK_GRAY).size(12.0));
									ui.label(egui::RichText::new(&comment.name).size(20.0));
									ui.label(&comment.body);
								});
							});
						}
					}
				});
			})
		});
	}
}
