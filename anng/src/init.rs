use core::fmt;
use nng_sys::nng_err;
use std::num::NonZeroUsize;

/// Thread limit configuration for NNG thread pools.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ThreadLimit {
    /// No limit on thread count.
    Unlimited,
    /// Specific limit on thread count.
    Limit(NonZeroUsize),
}

/// Configuration for a NNG thread pool.
///
/// # Constraints
///
/// When `max` is [`ThreadLimit::Limit`], it must be greater than or equal to `num`.
/// The [`init_nng`] function validates this and returns [`InitError::Invalid`] if violated.
#[derive(Debug, Copy, Clone)]
pub struct ThreadPoolConfig {
    /// Number of threads to create initially.
    pub num: NonZeroUsize,
    /// Maximum thread count. [`ThreadLimit::Unlimited`] removes the cap.
    pub max: ThreadLimit,
}

/// Configuration parameters for NNG library initialization.
///
/// All fields use `None` to indicate "use NNG's default value".
///
/// # Defaults
///
/// The default values are declared [within NNG](https://github.com/nanomsg/nng/blob/main/src/core/init.c)
#[derive(Debug, Default, Copy, Clone)]
pub struct NngConfig {
    /// Task queue threads. `None` = use NNG defaults.
    pub task_threads: Option<ThreadPoolConfig>,
    /// Expiration threads. `None` = use NNG defaults.
    pub expire_threads: Option<ThreadPoolConfig>,
    /// Poller threads. `None` = use NNG defaults.
    pub poller_threads: Option<ThreadPoolConfig>,
    /// Number of resolver threads. `None` = use NNG default.
    pub num_resolver_threads: Option<NonZeroUsize>,
}

/// Error returned by [`init_nng`] when initialization fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitError {
    /// Invalid configuration parameter.
    Invalid(String),
    /// NNG was already initialized with configuration parameters.
    AlreadyInitialized,
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(msg) => write!(f, "invalid config: {msg}"),
            Self::AlreadyInitialized => write!(f, "NNG already initialized"),
        }
    }
}

impl std::error::Error for InitError {}

/// Initialize the NNG library.
///
/// # Optional
///
/// Calling this function is **optional**. NNG is automatically initialized
/// with default settings when the first socket is created. Use this function
/// _only_ if you need to customize thread pool sizes **before** creating any
/// sockets.
///
/// NNG internally tracks initialization:
/// - `init_nng(None)` can be called multiple times
/// - `init_nng(Some(config))` returns [`InitError::AlreadyInitialized`] if NNG was
///   already initialized. *Configuration can only be set on first init*.
///
/// # Errors
///
/// - [`InitError::Invalid`] - A configuration value is out of range or inconsistent.
/// - [`InitError::AlreadyInitialized`] - Config provided after initialization.
pub fn init_nng(config: Option<NngConfig>) -> Result<(), InitError> {
    let result = match config {
        Some(cfg) => {
            // Validate num <= max constraints
            if let Some(ref pool) = cfg.task_threads {
                validate_pool_config("task_threads", pool)?;
            }
            if let Some(ref pool) = cfg.expire_threads {
                validate_pool_config("expire_threads", pool)?;
            }
            if let Some(ref pool) = cfg.poller_threads {
                validate_pool_config("poller_threads", pool)?;
            }

            let params = nng_sys::nng_init_params {
                num_task_threads: pool_num_to_i16("task_threads.num", cfg.task_threads)?,
                max_task_threads: pool_max_to_i16("task_threads.max", cfg.task_threads)?,
                num_expire_threads: pool_num_to_i16("expire_threads.num", cfg.expire_threads)?,
                max_expire_threads: pool_max_to_i16("expire_threads.max", cfg.expire_threads)?,
                num_poller_threads: pool_num_to_i16("poller_threads.num", cfg.poller_threads)?,
                max_poller_threads: pool_max_to_i16("poller_threads.max", cfg.poller_threads)?,
                num_resolver_threads: to_i16("num_resolver_threads", cfg.num_resolver_threads)?,
            };
            // SAFETY: params is valid and properly initialized
            unsafe { nng_sys::nng_init(&params) }
        }
        None => {
            // SAFETY: null params means use defaults
            unsafe { nng_sys::nng_init(std::ptr::null()) }
        }
    };

    match result {
        nng_err::NNG_OK => Ok(()),
        nng_err::NNG_EBUSY => Err(InitError::AlreadyInitialized),
        err => unreachable!("nng_init never returns {err}"),
    }
}

fn to_i16(name: &'static str, v: Option<NonZeroUsize>) -> Result<i16, InitError> {
    match v {
        None => Ok(0),
        Some(n) => nonzero_to_i16(name, n),
    }
}

fn nonzero_to_i16(name: &'static str, n: NonZeroUsize) -> Result<i16, InitError> {
    let val = n.get();
    if val > i16::MAX as usize {
        Err(InitError::Invalid(format!(
            "{name} value {val} exceeds max {}",
            i16::MAX
        )))
    } else {
        Ok(val as i16)
    }
}

fn limit_to_i16(name: &'static str, limit: ThreadLimit) -> Result<i16, InitError> {
    match limit {
        ThreadLimit::Unlimited => Ok(-1),
        ThreadLimit::Limit(n) => nonzero_to_i16(name, n),
    }
}

fn pool_num_to_i16(name: &'static str, pool: Option<ThreadPoolConfig>) -> Result<i16, InitError> {
    match pool {
        None => Ok(0),
        Some(p) => nonzero_to_i16(name, p.num),
    }
}

fn pool_max_to_i16(name: &'static str, pool: Option<ThreadPoolConfig>) -> Result<i16, InitError> {
    match pool {
        None => Ok(0),
        Some(p) => limit_to_i16(name, p.max),
    }
}

fn validate_pool_config(name: &'static str, pool: &ThreadPoolConfig) -> Result<(), InitError> {
    if let ThreadLimit::Limit(max) = pool.max {
        if pool.num.get() > max.get() {
            return Err(InitError::Invalid(format!(
                "{name}.num ({}) exceeds {name}.max ({})",
                pool.num.get(),
                max.get()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    #[test]
    fn test_nonzero_to_i16() {
        // Valid values
        assert_eq!(nonzero_to_i16("f", nz(100)), Ok(100));
        assert_eq!(nonzero_to_i16("f", nz(i16::MAX as usize)), Ok(i16::MAX));

        // Exceeds max
        let result = nonzero_to_i16("test_field", nz(i16::MAX as usize + 1));
        assert!(matches!(result, Err(InitError::Invalid(msg)) if msg.contains("test_field") && msg.contains("exceeds max")));
    }

    #[test]
    fn test_to_i16() {
        assert_eq!(to_i16("f", None), Ok(0));
        assert_eq!(to_i16("f", Some(nz(50))), Ok(50));
    }

    #[test]
    fn test_limit_to_i16() {
        // Unlimited
        assert_eq!(limit_to_i16("f", ThreadLimit::Unlimited), Ok(-1));
        // Valid limit
        assert_eq!(limit_to_i16("f", ThreadLimit::Limit(nz(42))), Ok(42));

        // Exceeds max
        let result = limit_to_i16("test_field", ThreadLimit::Limit(nz(i16::MAX as usize + 1)));
        assert!(matches!(result, Err(InitError::Invalid(msg)) if msg.contains("test_field")));
    }

    #[test]
    fn test_pool_num_to_i16() {
        assert_eq!(pool_num_to_i16("f", None), Ok(0));

        let pool = ThreadPoolConfig { num: nz(8), max: ThreadLimit::Unlimited };
        assert_eq!(pool_num_to_i16("f", Some(pool)), Ok(8));
    }

    #[test]
    fn test_pool_max_to_i16() {
        assert_eq!(pool_max_to_i16("f", None), Ok(0));

        let unlimited = ThreadPoolConfig { num: nz(4), max: ThreadLimit::Unlimited };
        assert_eq!(pool_max_to_i16("f", Some(unlimited)), Ok(-1));

        let limited = ThreadPoolConfig { num: nz(4), max: ThreadLimit::Limit(nz(16)) };
        assert_eq!(pool_max_to_i16("f", Some(limited)), Ok(16));
    }

    #[test]
    fn test_validate_pool_config() {
        // Unlimited max always ok
        let unlimited = ThreadPoolConfig { num: nz(1000), max: ThreadLimit::Unlimited };
        assert!(validate_pool_config("t", &unlimited).is_ok());

        // num == max ok
        let equal = ThreadPoolConfig { num: nz(10), max: ThreadLimit::Limit(nz(10)) };
        assert!(validate_pool_config("t", &equal).is_ok());

        // num < max ok
        let less = ThreadPoolConfig { num: nz(5), max: ThreadLimit::Limit(nz(10)) };
        assert!(validate_pool_config("t", &less).is_ok());

        // num > max error
        let exceeds = ThreadPoolConfig { num: nz(15), max: ThreadLimit::Limit(nz(10)) };
        let result = validate_pool_config("task_threads", &exceeds);
        assert!(matches!(result, Err(InitError::Invalid(msg)) 
            if msg.contains("task_threads.num") && msg.contains("15") 
            && msg.contains("task_threads.max") && msg.contains("10")));
    }
}
