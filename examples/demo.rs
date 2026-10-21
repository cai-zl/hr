//! Minimal demo service: `cargo run --example demo`, then
//! `curl localhost:3000/user/1` and `curl localhost:3000/user/0`.
use axum::extract::Path;
use axum::{Router, routing::get};
use hr::{Hr, HrError, HrPage, HrResult};

#[derive(serde::Serialize)]
struct User {
    id: u32,
    name: String,
}

async fn get_user(Path(id): Path<u32>) -> HrResult<User> {
    if id == 0 {
        return Err(HrError::new(404, "user not found"));
    }
    Ok(Hr::ok(User { id, name: "alice".into() }))
}

async fn list_users() -> Hr<HrPage<User>> {
    let users = vec![User { id: 1, name: "alice".into() }, User { id: 2, name: "bob".into() }];
    Hr::page(users, 2, 1, 20)
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/user/{id}", get(get_user))
        .route("/users", get(list_users));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("listening on http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}
