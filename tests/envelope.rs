use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::{Router, routing::get};
use http_body_util::BodyExt;
use tower::ServiceExt;

use hr::{Hr, HrError, HrPage, HrResult};

#[derive(serde::Serialize)]
struct User {
    id: u32,
    name: String,
}

fn alice() -> User {
    User {
        id: 1,
        name: "alice".into(),
    }
}

async fn result_ok() -> HrResult<User> {
    Ok(Hr::ok(alice()))
}

async fn result_err() -> HrResult<User> {
    Err("boom".into())
}

async fn anyhow_route() -> HrResult<User> {
    let found: anyhow::Result<User> = Err(anyhow::anyhow!("db down"));
    let user = found?;
    Ok(Hr::ok(user))
}

async fn status_route() -> HrResult<User> {
    Err(StatusCode::NOT_FOUND.into())
}

async fn page_route() -> Hr<HrPage<User>> {
    Hr::page(vec![alice()], 2, 1, 1)
}

fn app() -> Router {
    Router::new()
        .route("/ok", get(|| async { Hr::ok(alice()) }))
        .route(
            "/ok-with",
            get(|| async { Hr::ok_with(alice(), "created") }),
        )
        .route("/err", get(|| async { Hr::<User>::err(1001, "no user") }))
        .route("/result-ok", get(result_ok))
        .route("/result-err", get(result_err))
        .route("/anyhow", get(anyhow_route))
        .route("/status", get(status_route))
        .route("/page", get(page_route))
}

async fn hit(uri: &str) -> (StatusCode, String, serde_json::Value) {
    let res = app()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&body).expect("body must be JSON");
    (status, content_type, json)
}

#[tokio::test]
async fn ok_envelope_exact_shape() {
    let (status, content_type, json) = hit("/ok").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("application/json"));
    assert_eq!(
        json,
        serde_json::json!({"code": 0, "message": "ok", "data": {"id": 1, "name": "alice"}})
    );
}

#[tokio::test]
async fn ok_with_custom_message() {
    let (_, _, json) = hit("/ok-with").await;
    assert_eq!(json["code"], 0);
    assert_eq!(json["message"], "created");
    assert!(json["data"].is_object());
}

#[tokio::test]
async fn err_omits_data_field() {
    let (_, _, json) = hit("/err").await;
    assert_eq!(
        json,
        serde_json::json!({"code": 1001, "message": "no user"})
    );
    assert!(json.get("data").is_none());
}

#[tokio::test]
async fn hr_result_ok_wraps_envelope() {
    let (_, _, json) = hit("/result-ok").await;
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"]["name"], "alice");
}

#[tokio::test]
async fn hr_result_err_via_from_str() {
    let (_, _, json) = hit("/result-err").await;
    assert_eq!(json, serde_json::json!({"code": 500, "message": "boom"}));
}

#[tokio::test]
async fn anyhow_error_maps_to_500() {
    let (_, _, json) = hit("/anyhow").await;
    assert_eq!(
        json,
        serde_json::json!({"code": 500, "message": "internal error"})
    );
}

#[tokio::test]
async fn status_code_maps_to_business_code() {
    let (_, _, json) = hit("/status").await;
    assert_eq!(
        json,
        serde_json::json!({"code": 404, "message": "Not Found"})
    );
}

#[tokio::test]
async fn page_shape() {
    let (_, _, json) = hit("/page").await;
    assert_eq!(
        json,
        serde_json::json!({
            "code": 0,
            "message": "ok",
            "data": {"list": [{"id": 1, "name": "alice"}], "total": 2, "page": 1, "pageSize": 1}
        })
    );
}

#[test]
fn hr_error_helpers() {
    assert_eq!(HrError::msg("x").code, 500);
    assert_eq!(HrError::new(7, "y").code, 7);
}

#[tokio::test]
async fn merge_all_combines_routers() {
    let r1 = Router::new().route("/a", get(|| async { Hr::ok(alice()) }));
    let r2 = Router::new().route("/b", get(|| async { Hr::<User>::err(1002, "b") }));
    let app = hr::merge_all([r1, r2]);
    let res = app
        .oneshot(Request::get("/b").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 1002);
}

#[tokio::test]
async fn merge_all_fallback_returns_envelope() {
    let app = hr::merge_all([Router::new()]);
    let res = app
        .oneshot(Request::get("/nope").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"code": 404, "message": "Not Found"})
    );
}

#[tokio::test]
async fn merge_all_wraps_405_with_real_status() {
    let app = hr::merge_all([Router::new().route("/a", get(|| async { Hr::ok(alice()) }))]);
    let res = app
        .oneshot(Request::post("/a").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"code": 405, "message": "Method Not Allowed"})
    );
}

#[test]
fn err_zero_code_becomes_500() {
    let json = serde_json::to_value(Hr::<()>::err(0, "x")).unwrap();
    assert_eq!(json, serde_json::json!({"code": 500, "message": "x"}));
}

#[test]
fn hr_and_hr_page_deserialize_roundtrip() {
    let ok = serde_json::json!({"code":0,"message":"ok","data":{"id":1}});
    let hr: hr::Hr<serde_json::Value> = serde_json::from_value(ok.clone()).unwrap();
    assert_eq!(serde_json::to_value(&hr).unwrap(), ok);

    let err = serde_json::json!({"code":500,"message":"boom"});
    let hr: hr::Hr<serde_json::Value> = serde_json::from_value(err.clone()).unwrap();
    assert_eq!(serde_json::to_value(&hr).unwrap(), err);

    let page = serde_json::json!({"list":[],"total":5,"page":1,"pageSize":20});
    let p: hr::HrPage<serde_json::Value> = serde_json::from_value(page.clone()).unwrap();
    assert_eq!(serde_json::to_value(&p).unwrap(), page);
}
