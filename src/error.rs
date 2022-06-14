use thiserror::Error;

/// The adsbx_browser error type.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// Tried to use an Aircraft that didn't have the required data
    #[error("{0}")]
    AircraftMissingData(String),
    #[error("{0}")]
    JsonLoadError(String),
    #[error("{0}")]
    ParallelMapError(String),
}
