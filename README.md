# hr — axum 统一 HTTP 响应体

一个极小的 axum 响应封装库。所有 handler 返回同一个 JSON 结构，AI 写的服务不再各写各的响应体：

```json
{"code":0,"message":"ok","data":{"id":1,"name":"alice"}}
```

```json
{"code":404,"message":"Not Found"}
```

**固定规则：**

- HTTP 状态码**恒为 200**，Content-Type 恒为 `application/json`
- 成败只看 body 里的 `code`：`0` = 成功，非 0 = 失败
- `data` 只在成功时出现；失败响应没有 `data` 字段（不是 `null`，是整个省略）

## 安装

```toml
[dependencies]
hr = { git = "ssh://git@git.cosmic.net.cn/cosmic/hr.git" }
```

## API 总览

| 条目 | 用途 |
|---|---|
| `Hr<T>` / `Hr::ok` / `Hr::ok_with` / `Hr::err` / `Hr::page` | 成功 / 自定义成功消息 / 业务失败 / 分页 |
| `HrError` / `HrError::new` / `HrError::msg` | 错误类型，配 `?` 使用 |
| `HrResult<T>` | handler 标准返回类型（别名） |
| `HrPage<T>` / `HrPage::new` | 分页数据结构 |
| `From<...> for HrError` | `StatusCode` / `anyhow::Error` / `String` / `&str` 自动转换 |

---

## `Hr<T>` — 响应信封

结构体字段是私有的，只能通过构造器创建，保证所有响应结构一致。

```rust
pub struct Hr<T> { /* code: i32, message: String, data: Option<T> — 均私有 */ }

impl<T: Serialize> Hr<T> {
    pub fn ok(data: T) -> Self;
    pub fn ok_with(data: T, message: impl Into<String>) -> Self;
    pub fn err(code: i32, message: impl Into<String>) -> Self;
    pub fn page(list: Vec<T>, total: u64, page: u64, page_size: u64) -> Hr<HrPage<T>>;
}

impl<T: Serialize> IntoResponse for Hr<T> { /* 恒 200 + JSON */ }
```

### `Hr::ok(data)` — 成功

```rust
async fn get_user() -> Hr<User> {
    Hr::ok(User { id: 1, name: "alice".into() })
}
```

```json
{"code":0,"message":"ok","data":{"id":1,"name":"alice"}}
```

### `Hr::ok_with(data, message)` — 成功 + 自定义消息

```rust
async fn create_user() -> Hr<User> {
    let user = save_user(new_user)?;
    Hr::ok_with(user, "创建成功")
}
```

```json
{"code":0,"message":"创建成功","data":{"id":2,"name":"bob"}}
```

### `Hr::err(code, message)` — 业务失败

`data` 不出现在响应里。HTTP 状态仍是 200。

handler 直接返回 `Hr<T>`（不是 `HrResult`）时用它：

```rust
async fn ping() -> Hr<()> {
    Hr::err(1001, "服务未初始化")
}
```

```json
{"code":1001,"message":"服务未初始化"}
```

### `Hr::page(list, total, page, page_size)` — 分页

等价于 `Hr::ok(HrPage::new(...))`。`total` 是总条数（不是总页数）。

```rust
async fn list_users() -> Hr<HrPage<User>> {
    let (users, total) = query_users(page, size);
    Hr::page(users, total, page, size)
}
```

```json
{
  "code": 0,
  "message": "ok",
  "data": {
    "list": [{"id":1,"name":"alice"}, {"id":2,"name":"bob"}],
    "total": 57,
    "page": 1,
    "page_size": 20
  }
}
```

---

## `HrResult<T>` — handler 标准返回类型

```rust
pub type HrResult<T> = Result<Hr<T>, HrError>;
```

**推荐所有 handler 都用它。**`Ok` 侧包好信封，`Err` 侧（含 `?` 传播）自动变成错误信封：

```rust
async fn get_user(Path(id): Path<u32>) -> HrResult<User> {
    if id == 0 {
        return Err(HrError::new(404, "user not found"));
    }
    let user = find_user(id)?;   // 任何可转换的错误都自动处理，见下节
    Ok(Hr::ok(user))
}
```

---

## `HrError` — 错误类型

```rust
pub struct HrError {
    pub code: i32,       // 业务码，默认建议用 HTTP 语义数字，如 404、500
    pub message: String,
}

impl HrError {
    pub fn new(code: i32, message: impl Into<String>) -> Self;
    pub fn msg(message: impl Into<String>) -> Self;   // 等价 new(500, message)
}

impl IntoResponse for HrError { /* 转 {"code":...,"message":"..."} */ }
```

### `HrError::new(code, message)` — 指定业务码

```rust
return Err(HrError::new(1001, "名字已存在"));
```

```json
{"code":1001,"message":"名字已存在"}
```

### `HrError::msg(message)` — 默认 500

```rust
return Err(HrError::msg("数据库连接失败"));
```

```json
{"code":500,"message":"数据库连接失败"}
```

### 自动转换（`From` 实现）—— `?` 和 `.into()` 直接用

| 来源 | 转换结果 |
|---|---|
| `&str` / `String` | `{"code":500,"message":"<字符串>"}` |
| `anyhow::Error` | `{"code":500,"message":"<err.to_string()>"}` |
| `axum::http::StatusCode` | `{"code":<状态码数字>,"message":"<标准原因短语>"}` |

```rust
async fn demo() -> HrResult<User> {
    let user = repo.find(id)?;               // anyhow::Error → {"code":500,...}
    let user = repo.find(id).map_err(|_| "没找到".into())?;  // &str → {"code":500,...}
    Err(StatusCode::NOT_FOUND.into())        // → {"code":404,"message":"Not Found"}
}
```

---

## `HrPage<T>` — 分页结构

```rust
pub struct HrPage<T> { /* list: Vec<T>, total: u64, page: u64, page_size: u64 — 均私有 */ }

impl<T> HrPage<T> {
    pub fn new(list: Vec<T>, total: u64, page: u64, page_size: u64) -> Self;
}
```

一般不直接用它，用 `Hr::page(...)` 即可。序列化形状固定为：

```json
{"list":[...],"total":57,"page":1,"page_size":20}
```

---

## 完整示例服务

`examples/demo.rs`：

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

9 个集成测试覆盖：成功/失败信封精确形状、`data` 字段省略、`HrResult` 两条路径、anyhow / StatusCode / &str 转换、分页形状。
