//! Unified success-envelope type [`ApiResponse<T>`].
//!
//! This is the Rust counterpart of `hwhkit.web.api_response.ApiResponse`
//! in `hwhkit-py` (see `hwhkit-py/hwhkit/web/responses.py`). The two
//! implementations are wire-compatible: the JSON shape
//!
//! ```text
//! { "code": 0, "message": "ok", "data": <T>, "trace_id": "..." }
//! ```
//!
//! round-trips between languages, so a service written in either stack
//! can call into the other without a translation layer.
//!
//! # Wire shape
//!
//! - `code` (`i32`): business status. `0` is the agreed-upon success
//!   sentinel; any non-zero value is a business-level error.
//! - `message` (`String`): short human-readable summary. Defaults to
//!   `"ok"` for [`ApiResponse::ok`] and is required for
//!   [`ApiResponse::err`].
//! - `data` (`Option<T>`): the payload. `None` on errors; `Some(_)` on
//!   success even when `T = ()`-like (the caller chooses).
//! - `trace_id` (`Option<String>`): optional trace correlation id. We
//!   keep it `Option` (unlike the harness placeholder, which used a bare
//!   `String`) so the field can be omitted from the wire when no tracing
//!   context is active — matching the Python side, which defaults to
//!   `None` when no OTel span is in scope.
//!
//! # Error code space
//!
//! Non-zero `code` values follow the same 6-digit `XYYZZZ` scheme used by
//! `hwhkit-py`'s `hwhkit.core.errors` module:
//!
//! - `X` — severity / category bucket (1xxxxx client, 2xxxxx server, …)
//! - `YY` — subsystem (auth, db, llm, …)
//! - `ZZZ` — concrete error within the subsystem
//!
//! This crate does not currently ship a registry of those codes — pick
//! values from the Python catalogue and keep them in sync. The type is
//! intentionally `i32` (not a `u32` or enum) to leave room for negative
//! codes used by some third-party gateways we proxy through.
//!
//! # RFC 7807 vs envelope
//!
//! For *HTTP-level* errors prefer [`crate::error_response::ApiError`],
//! which emits `application/problem+json` per RFC 7807. `ApiResponse`
//! is the **business** envelope: it always returns HTTP 200 and signals
//! success/failure via the `code` field. Most internal-RPC-style routes
//! want this; public REST endpoints typically want RFC 7807.
//!
//! # Changelog
//!
//! - *unreleased* — initial cross-language port of the Python envelope.
//!   Added to unblock `hwhkit-bench` B-06, which previously kept a
//!   harness-local placeholder.

use serde::{Deserialize, Serialize};

/// Unified success-envelope returned by HTTP handlers that opt in to the
/// `ApiResponse` convention.
///
/// See the [module docs](self) for the wire shape and error-code rules.
///
/// # Example
///
/// ```
/// use hwhkit_core::ApiResponse;
///
/// let env = ApiResponse::ok(42u32).with_trace_id("abc123");
/// assert_eq!(env.code, 0);
/// assert_eq!(env.data, Some(42));
/// assert_eq!(env.trace_id.as_deref(), Some("abc123"));
///
/// let json = serde_json::to_string(&env).unwrap();
/// let back: ApiResponse<u32> = serde_json::from_str(&json).unwrap();
/// assert_eq!(back.data, Some(42));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ApiResponse<T> {
    /// Business status code. `0` means success; non-zero values follow
    /// the 6-digit `XYYZZZ` scheme shared with `hwhkit-py`'s
    /// `hwhkit.core.errors` catalogue.
    pub code: i32,
    /// Short human-readable summary. `"ok"` for successes; the
    /// caller-supplied message for errors.
    pub message: String,
    /// Optional payload. Always `None` on the error path.
    pub data: Option<T>,
    /// Optional trace-correlation id (matches the Python side, which
    /// fills this from the current OTel span when one is active).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trace_id: Option<String>,
}

impl<T> ApiResponse<T> {
    /// The agreed-upon success sentinel for [`Self::code`]. Mirrors the
    /// `code: int = 0` default in `hwhkit-py`.
    pub const SUCCESS_CODE: i32 = 0;

    /// Construct a successful envelope wrapping `data`.
    ///
    /// `code` is set to [`Self::SUCCESS_CODE`] (`0`) and `message` to
    /// `"ok"` — same defaults as the Python side. `trace_id` is left
    /// empty; attach one with [`Self::with_trace_id`].
    pub fn ok(data: T) -> Self {
        Self {
            code: Self::SUCCESS_CODE,
            message: "ok".to_string(),
            data: Some(data),
            trace_id: None,
        }
    }

    /// Construct an error envelope with a business `code` and human
    /// `message`. `data` is `None`.
    ///
    /// `code` *should* be non-zero — a zero `code` means success by
    /// convention. The constructor does not enforce this so callers can
    /// faithfully reflect upstream payloads that violate the convention.
    pub fn err(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
            trace_id: None,
        }
    }

    /// Attach a trace-correlation id, consuming and returning `self` so
    /// the call chains naturally:
    ///
    /// ```ignore
    /// ApiResponse::ok(payload).with_trace_id(req_id)
    /// ```
    #[must_use]
    pub fn with_trace_id(mut self, id: impl Into<String>) -> Self {
        self.trace_id = Some(id.into());
        self
    }

    /// `true` when [`Self::code`] equals [`Self::SUCCESS_CODE`]. Use this
    /// instead of comparing the field directly so the success convention
    /// stays in one place.
    pub fn is_success(&self) -> bool {
        self.code == Self::SUCCESS_CODE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ok_uses_success_defaults() {
        let env = ApiResponse::ok(42u32);
        assert_eq!(env.code, 0);
        assert_eq!(env.code, ApiResponse::<u32>::SUCCESS_CODE);
        assert_eq!(env.message, "ok");
        assert_eq!(env.data, Some(42));
        assert!(env.trace_id.is_none());
        assert!(env.is_success());
    }

    #[test]
    fn ok_serialize_shape_matches_python() {
        // Wire-compat with hwhkit-py: code/message/data fields present,
        // trace_id omitted when None.
        let env = ApiResponse::ok(json!({ "id": 1 }));
        let serialized: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&env).unwrap()).unwrap();
        assert_eq!(serialized["code"], 0);
        assert_eq!(serialized["message"], "ok");
        assert_eq!(serialized["data"], json!({ "id": 1 }));
        assert!(serialized.get("trace_id").is_none());
    }

    #[test]
    fn err_carries_code_and_message_no_data() {
        let env: ApiResponse<u32> = ApiResponse::err(100_404, "user not found");
        assert_eq!(env.code, 100_404);
        assert_eq!(env.message, "user not found");
        assert!(env.data.is_none());
        assert!(!env.is_success());
    }

    #[test]
    fn with_trace_id_attaches_id() {
        let env = ApiResponse::ok(()).with_trace_id("trace-abc");
        assert_eq!(env.trace_id.as_deref(), Some("trace-abc"));

        // Chained on the error path too.
        let env: ApiResponse<()> = ApiResponse::err(200_001, "boom").with_trace_id("trace-xyz");
        assert_eq!(env.trace_id.as_deref(), Some("trace-xyz"));
    }

    #[test]
    fn with_trace_id_present_serializes() {
        let env = ApiResponse::ok(1u32).with_trace_id("tid");
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&env).unwrap()).unwrap();
        assert_eq!(value["trace_id"], "tid");
    }

    #[test]
    fn deserialize_round_trip() {
        let original = ApiResponse::ok(vec![1u32, 2, 3]).with_trace_id("rt");
        let json = serde_json::to_string(&original).unwrap();
        let back: ApiResponse<Vec<u32>> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.code, original.code);
        assert_eq!(back.message, original.message);
        assert_eq!(back.data, original.data);
        assert_eq!(back.trace_id, original.trace_id);
    }

    #[test]
    fn deserialize_accepts_missing_trace_id() {
        // Wire-compat: a peer that omits trace_id (Python default) must
        // deserialize cleanly into Option::None.
        let env: ApiResponse<u32> =
            serde_json::from_str(r#"{"code":0,"message":"ok","data":7}"#).unwrap();
        assert_eq!(env.data, Some(7));
        assert!(env.trace_id.is_none());
    }

    #[test]
    fn code_zero_is_success_semantics() {
        // Hand-rolled (e.g. from a peer's payload) — confirm we treat
        // code=0 as success regardless of how the value was constructed.
        let env = ApiResponse::<u32> {
            code: 0,
            message: "anything".into(),
            data: Some(1),
            trace_id: None,
        };
        assert!(env.is_success());

        let env = ApiResponse::<u32> {
            code: 1,
            message: "ok".into(),
            data: Some(1),
            trace_id: None,
        };
        assert!(!env.is_success());
    }
}
