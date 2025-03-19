use std::{error::Error, fmt, future::Future, hash::Hash};

use serde::de::DeserializeOwned;

/// The `Fetcher` is responsible for fetching resources (likely from a remote server) when a key is not present in the
/// cache, or needs to be revalidated.
pub trait Fetcher: Send + Sync + 'static {
	/// The fetcher's response type; optionally a wrapper around the response body `T`.
	///
	/// In the **vast** majority of cases, you'll probably want to define this as:
	/// ```
	/// type Response<T: Send + Sync + 'static> = T;
	/// ```
	///
	/// However, if you require other data from the response besides its deserialized body, like response headers, you
	/// may want to wrap the body in a type that allows you to access this additional data:
	/// ```
	/// use std::ops::Deref;
	///
	/// use serde::de::DeserializeOwned;
	/// # mod http {
	/// # 	pub struct HeaderMap;
	/// # 	impl std::ops::Index<u32> for HeaderMap {
	/// # 		type Output = &'static str;
	/// # 		fn index(&self, _: u32) -> &&'static str {
	/// # 			&"Deep Thought"
	/// # 		}
	/// # 	}
	/// # 	pub mod header {
	/// # 		pub const SERVER: u32 = 0;
	/// # 	}
	/// # }
	///
	/// struct MyResponse<T> {
	/// 	pub headers: http::HeaderMap,
	/// 	pub body: T
	/// }
	///
	/// // For usability, you should let your response type `Deref` to its body:
	/// impl<T> Deref for MyResponse<T> {
	/// 	type Target = T;
	/// 	fn deref(&self) -> &Self::Target {
	/// 		&self.body
	/// 	}
	/// }
	///
	/// # type Error = serde_json::Error;
	/// struct Fetcher;
	/// impl swr::Fetcher for Fetcher {
	/// 	type Response<T: Send + Sync + 'static> = MyResponse<T>;
	/// 	type Error = Error;
	/// 	type Key = String;
	///
	/// 	async fn fetch<T: DeserializeOwned + Send + Sync + 'static>(&self, key: &Self::Key) -> Result<Self::Response<T>, Self::Error> {
	/// 		# let _ = stringify!{
	/// 		...
	/// 		# };
	/// 		# Ok(MyResponse { headers: http::HeaderMap, body: serde_json::from_str("42")? })
	/// 	}
	/// }
	///
	/// # #[tokio::main]
	/// # async fn main() {
	/// # type Todo = u32;
	/// # let hook = swr::hook::MockHook::default();
	/// let swr = swr::new_in(Fetcher, swr::runtime::Tokio, hook);
	///
	/// # loop {
	/// let result = swr.get::<u32, _>("/answer");
	/// # if result.loading { continue; }
	/// let response = result.data.unwrap();
	/// assert_eq!(**response, 42);
	/// assert_eq!(response.headers[http::header::SERVER], "Deep Thought");
	/// # break;
	/// # }
	/// # }
	/// ```
	type Response<T: Send + Sync + 'static>: Send + Sync + 'static;

	/// The error type returned when a fetch fails.
	type Error: Error + Send + Sync;

	/// This fetcher's 'key' type.
	///
	/// This can be, for example, a [`String`] representing a URL path, which the fetcher can use to send an HTTP
	/// request.
	///
	/// You can also implement more complex, struct-based keys:
	/// ```
	/// use std::borrow::Borrow;
	///
	/// use serde::de::DeserializeOwned;
	///
	/// #[derive(Debug, Clone, Hash, PartialEq, Eq)]
	/// struct CustomKey {
	/// 	path: String,
	/// 	has_query_params: bool
	/// }
	///
	/// impl CustomKey {
	/// 	pub fn new(path: impl Into<String>) -> Self {
	/// 		Self {
	/// 			path: path.into(),
	/// 			has_query_params: false
	/// 		}
	/// 	}
	///
	/// 	pub fn query(mut self, name: &str, value: &str) -> Self {
	/// 		self.path.push(if self.has_query_params { '&' } else { '?' });
	/// 		self.path.push_str(name);
	/// 		self.path.push('=');
	/// 		self.path.push_str(value);
	/// 		self
	/// 	}
	/// }
	///
	/// impl From<&CustomKey> for CustomKey {
	/// 	fn from(value: &CustomKey) -> Self {
	/// 		value.clone()
	/// 	}
	/// }
	///
	/// impl From<&str> for CustomKey {
	/// 	fn from(value: &str) -> Self {
	/// 		Self {
	/// 			path: value.to_string(),
	/// 			has_query_params: value.contains("?")
	/// 		}
	/// 	}
	/// }
	///
	/// impl Borrow<str> for CustomKey {
	/// 	fn borrow(&self) -> &str {
	/// 		&self.path
	/// 	}
	/// }
	///
	/// # type Error = serde_json::Error;
	/// struct Fetcher;
	/// impl swr::Fetcher for Fetcher {
	/// 	type Response<T: Send + Sync + 'static> = T;
	/// 	type Error = Error;
	/// 	type Key = CustomKey;
	///
	/// 	async fn fetch<T: DeserializeOwned + Send + Sync + 'static>(&self, key: &Self::Key) -> Result<Self::Response<T>, Self::Error> {
	/// 		# let _ = stringify!{
	/// 		...
	/// 		# };
	/// 		# serde_json::from_str("0")
	/// 	}
	/// }
	///
	/// # #[tokio::main]
	/// # async fn main() {
	/// # type Todo = u32;
	/// # let hook = swr::hook::MockHook::default();
	/// let swr = swr::new_in(Fetcher, swr::runtime::Tokio, hook);
	///
	/// let string_key_res = swr.get::<Todo, _>("/todos/1");
	/// let custom_key_res = swr.get::<Vec<Todo>, _>(&CustomKey::new("/todos").query("sort", "asc"));
	/// # }
	/// ```
	///
	/// For more details on implementing a custom complex key type, see the [`complex-key` example][ck].
	///
	/// [ck]: https://github.com/pykeio/swr/blob/main/examples/complex-key.rs
	type Key: fmt::Debug + Clone + Hash + Eq + Send + Sync;

	/// Fetches the resource using the given key, deserializing the response body as type `T`.
	fn fetch<T: DeserializeOwned + Send + Sync + 'static>(&self, key: &Self::Key) -> impl Future<Output = Result<Self::Response<T>, Self::Error>> + Send;
}

#[cfg(test)]
pub(crate) mod mock {
	use std::{
		fmt,
		marker::PhantomData,
		sync::{
			Arc,
			atomic::{AtomicUsize, Ordering}
		},
		time::Duration
	};

	use tokio::time::sleep;

	#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
	pub enum Key {
		Basic,
		Delayed(Duration),
		AlwaysError,
		ErrorNTimes(usize)
	}

	impl From<&Key> for Key {
		fn from(value: &Key) -> Self {
			*value
		}
	}

	#[derive(Debug, Default)]
	pub struct Error;

	impl fmt::Display for Error {
		fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
			f.write_str("error")
		}
	}

	impl std::error::Error for Error {}

	#[derive(Default)]
	struct FetcherInner {
		fetch_count: AtomicUsize,
		error_count: AtomicUsize
	}

	pub struct Fetcher<E = Error>(Arc<FetcherInner>, PhantomData<E>);

	impl<E> Default for Fetcher<E> {
		fn default() -> Self {
			Fetcher(Arc::default(), PhantomData)
		}
	}

	impl<E> Clone for Fetcher<E> {
		fn clone(&self) -> Self {
			Fetcher(Arc::clone(&self.0), PhantomData)
		}
	}

	impl Fetcher<Error> {
		pub fn new() -> Self {
			Fetcher(Arc::default(), PhantomData)
		}
	}

	impl<E> Fetcher<E> {
		pub fn fetch_count(&self) -> usize {
			self.0.fetch_count.load(Ordering::Acquire)
		}

		pub fn reset(&self) {
			self.0.fetch_count.store(0, Ordering::Release);
			self.0.error_count.store(0, Ordering::Release);
		}
	}

	impl<E: std::error::Error + Default + Sync + Send + 'static> super::Fetcher for Fetcher<E> {
		type Key = Key;
		type Error = E;
		type Response<T: Send + Sync + 'static> = T;

		async fn fetch<T: serde::de::DeserializeOwned + Send + Sync + 'static>(&self, key: &Self::Key) -> Result<Self::Response<T>, Self::Error> {
			self.0.fetch_count.fetch_add(1, Ordering::AcqRel);

			match key {
				Key::Basic => serde_json::from_str("42").map_err(|_| E::default()),
				Key::Delayed(delay) => {
					sleep(*delay).await;
					serde_json::from_str("42").map_err(|_| E::default())
				}
				Key::AlwaysError => Err(E::default()),
				Key::ErrorNTimes(n) => {
					let err_count = self.0.error_count.fetch_add(1, Ordering::AcqRel);
					if err_count == *n {
						serde_json::from_str("42").map_err(|_| E::default())
					} else {
						Err(E::default())
					}
				}
			}
		}
	}
}
