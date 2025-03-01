use std::{any::TypeId, fmt, sync::Arc};

use crate::fetcher::Fetcher;

/// Any error that can result from requesting a key.
pub enum Error<F: Fetcher> {
	/// An error occurred when attempting to fetch the key.
	Fetcher(Arc<F::Error>),
	/// The type contained in the cache does not match the requested type.
	MismatchedType(MismatchedTypeError)
}

impl<F: Fetcher> fmt::Debug for Error<F>
where
	F::Error: fmt::Debug
{
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Fetcher(e) => f.debug_tuple("Error::Fetcher").field(e).finish(),
			Self::MismatchedType(e) => f.debug_tuple("Error::MismatchedType").field(e).finish()
		}
	}
}

impl<F: Fetcher> fmt::Display for Error<F> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Fetcher(e) => {
				f.write_str("Failed to fetch: ")?;
				fmt::Display::fmt(e, f)
			}
			Self::MismatchedType(e) => fmt::Display::fmt(e, f)
		}
	}
}

impl<F: Fetcher> Clone for Error<F> {
	fn clone(&self) -> Self {
		match self {
			Self::Fetcher(e) => Self::Fetcher(Arc::clone(e)),
			Self::MismatchedType(e) => Self::MismatchedType(e.clone())
		}
	}
}

impl<F: Fetcher> std::error::Error for Error<F> {}

/// An error caused when the type contained in the cache does not match the requested type.
///
/// This often occurs when two parts of your code request the same key, but with different response types.
#[derive(Clone, Debug)]
pub struct MismatchedTypeError {
	/// The ID of the type contained in the cache.
	pub contained_type: TypeId,
	/// The ID of the type which was requested.
	pub wanted_type: TypeId,
	#[cfg(debug_assertions)]
	pub(crate) contained_type_name: &'static str,
	#[cfg(debug_assertions)]
	pub(crate) wanted_type_name: &'static str
}

impl MismatchedTypeError {
	/// Returns the name of the type contained in the cache, or `None` if SWR was not compiled with debug assertions
	/// (`--release`).
	#[inline]
	pub fn contained_type_name(&self) -> Option<&'static str> {
		#[cfg(debug_assertions)]
		return Some(self.contained_type_name);
		#[cfg(not(debug_assertions))]
		None
	}

	/// Returns the name of the requested type, or `None` if SWR was not compiled with debug assertions (`--release`).
	#[inline]
	pub fn wanted_type_name(&self) -> Option<&'static str> {
		#[cfg(debug_assertions)]
		return Some(self.wanted_type_name);
		#[cfg(not(debug_assertions))]
		None
	}
}

impl fmt::Display for MismatchedTypeError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("Data type mismatch")?;
		#[cfg(debug_assertions)]
		{
			f.write_str(" - cache contains a value of type `")?;
			f.write_str(self.contained_type_name)?;
			f.write_str("`, but tried to retrieve a value of type `")?;
			f.write_str(self.wanted_type_name)?;
			f.write_str("`.")?;
		}
		Ok(())
	}
}

impl std::error::Error for MismatchedTypeError {}
