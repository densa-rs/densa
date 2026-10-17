//! Benchmarks only; see `benches/`. A separate crate so criterion's dependency tree never
//! reaches the libraries' dev-dependencies (it does not build under Miri's cross target).
//!
//! Every bench pins its thread to a performance core: on hybrid CPUs unpinned results for
//! identical code differed 2-3x between runs.
