# hr — axum 统一 HTTP 响应体

所有 handler 返回同一个结构，AI 写的服务不再各写各的响应体：

```json
{"code":0,"message":"ok","data":{"id":1}}
{"code":404,"message":"Not Found"}
```

HTTP 状态恒为 200，成败看 `code`（0 = 成功）。

## 用法

```rust
use axum::{Router, routing::get};
use hr::{Hr, HrError, HrResult};

#[derive(serde::Serialize)]
struct User { id: u32, name: String }

async fn get_user(Path(id): Path<u32>) -> HrResult<User> {
    let user = find_user(id)?;              // anyhow::Error → {"code":500,...}
    Ok(Hr::ok(user))                        // {"code":0,"message":"ok","data":{...}}
}

async fn create_user() -> HrResult<User> {
    Err(HrError::new(1001, "名字已存在"))     // 自定义业务码
}

async fn list_users() -> Hr<HrPage<User>> {
    Hr::page(users, total, page, page_size) // data = {list,total,page,page_size}
}
```

`Err(...)` 直接接受的类型：`HrError`、`StatusCode`、`anyhow::Error`、`String`、`&str`（`"boom".into()` 即可）。

## 完整示例

```bash
cargo run --example demo
curl localhost:3000/user/1   # {"code":0,"message":"ok","data":{"id":1,"name":"alice"}}
curl localhost:3000/user/0   # {"code":404,"message":"user not found"}
curl localhost:3000/users    # 分页
```

## 给 AI 的提示（贴到你的服务项目里）

> 本项目所有 HTTP handler 必须使用 `hr` 库返回响应：成功返回 `Ok(Hr::ok(data))`（或 `Hr::page(list, total, page, page_size)` 分页），失败返回 `Err(...)`，handler 签名用 `HrResult<T>` 或 `Hr<T>`。不要手写 JSON 响应结构，不要改 HTTP 状态码。

## 测试

```bash
cargo test
```
