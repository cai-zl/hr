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

- 响应 JSON 键统一驼峰（camelCase）；当前唯一多词键是分页的 `pageSize`

## 0.2 模块（feature 默认全开）

| feature | 内容 | 依赖 |
|---|---|---|
| `pg` | PostgreSQL 连接池 | sqlx 0.9 |
| `redis` | Redis 连接池 | deadpool-redis 0.23 |
| `nacos` | Nacos 配置中心 + 服务注册 | nacos-sdk 0.3 |
| `boot` | 一键启动：配置 → 连接 → 注册 → 服务 → 停机反注册 | 前三者 + toml + tokio |

只要响应信封？关掉默认 feature：`default-features = false`。
## 安装

```toml
[dependencies]
hr = { git = "ssh://git@github.com/cai-zl/hr.git" }
```

## API 总览

| 条目 | 用途 |
|---|---|
| `Hr<T>` / `Hr::ok` / `Hr::ok_with` / `Hr::err` / `Hr::page` | 成功 / 自定义成功消息 / 业务失败 / 分页 |
| `HrError` / `HrError::new` / `HrError::msg` | 错误类型，配 `?` 使用 |
| `HrResult<T>` | handler 标准返回类型（别名） |
| `HrPage<T>` / `HrPage::new` | 分页数据结构 |
| `From<...> for HrError` | `StatusCode` / `anyhow::Error` / `String` / `&str` 自动转换 |

| `hr::merge_all([...])` | 合并各业务模块路由 |
| `pg::connect` / `redis::connect` | 数据库 / Redis 连接池 |
| `nacos::Nacos` | 配置中心读取 + 服务注册/注销 |
| `boot::run_env` / `boot::run` | 一键启动整套服务（见下文） |
| `AppState`（boot） | 全家桶状态：config / pg / redis / nacos |
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
    "pageSize": 20
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

| `sqlx::Error`（pg feature） | `{"code":500,...}` |
| `deadpool_redis::PoolError` / `CreatePoolError` / `redis::RedisError`（redis feature） | `{"code":500,...}` |
| `nacos_sdk::api::error::Error`（nacos feature） | `{"code":500,...}` |
| `toml::de::Error`（boot feature） | `{"code":500,...}` |
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
{"list":[...],"total":57,"page":1,"pageSize":20}
```

---

## `pg` 模块 — PostgreSQL 连接池

```rust
pub use sqlx;   // 重新导出，业务代码无需直接依赖 sqlx

pub async fn connect(url: &str) -> Result<sqlx::PgPool, sqlx::Error>;
// max_connections = 10，acquire 超时 5s
```

```rust
let pool = hr::pg::connect("postgres://user:pass@127.0.0.1:5432/mydb").await?;

let name: Option<(String,)> = sqlx::query_as::<_, (String,)>("select name from t_user where id = $1")
    .bind(1)
    .fetch_optional(&pool)
    .await
    .map_err(hr::HrError::from)?;   // sqlx::Error → {"code":500,...}
```

## `redis` 模块 — Redis 连接池

```rust
pub use deadpool_redis;   // 重新导出

pub fn connect(url: &str, max_size: usize)
    -> Result<deadpool_redis::Pool, deadpool_redis::CreatePoolError>;
```

```rust
let pool = hr::redis::connect("redis://127.0.0.1:6379", 10)?;

let mut conn = pool.get().await.map_err(hr::HrError::from)?;
let pong: String = deadpool_redis::redis::cmd("PING")
    .query_async(&mut *conn)
    .await
    .map_err(hr::HrError::from)?;
```

## `nacos` 模块 — 配置中心 + 服务注册

```rust
pub struct Nacos { /* Clone；克隆只是引用计数 */ }

pub fn connect(server_addr: &str, namespace: &str) -> Result<Nacos, HrError>;
pub fn from_env() -> Result<Nacos, HrError>;
pub fn parse_config<T: DeserializeOwned>(text: &str) -> Result<T, HrError>;

impl Nacos {
    pub fn get_config(&self, data_id: &str, group: &str) -> Result<Option<String>, HrError>;
    pub fn load_config<T: DeserializeOwned>(&self, data_id: &str, group: &str) -> Result<Option<T>, HrError>;
    pub fn register(&self, service: &str, ip: &str, port: i32, group: &str) -> Result<(), HrError>;
    pub fn deregister(&self, service: &str, ip: &str, port: i32, group: &str) -> Result<(), HrError>;
}
```

### 环境变量

| 变量 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `NACOS_SERVER_ADDR` | ✅ | — | 如 `127.0.0.1:8848`；缺失时 `from_env` 直接报错 |
| `NACOS_NAMESPACE` | | `public` | |
| `NACOS_USERNAME` / `NACOS_PASSWORD` | | — | 两者都设置才启用鉴权 |
| `NACOS_DATA_ID` | | 运行时二进制名 | `boot::run_env` 读配置用；二进制改名部署时显式覆盖 |
| `NACOS_GROUP` | | `DEFAULT_GROUP` | 读配置与服务注册共用 |

### 用法

```rust
// 配置中心：读 TOML 并解析；配置不存在返回 Ok(None)
let cfg: Option<MyConfig> = nacos.load_config("my-app", "DEFAULT_GROUP")?;

// 服务注册（临时实例，进程退出或超时后 Nacos 自动摘除）
nacos.register("my-app", "192.168.1.10", 3000, "DEFAULT_GROUP")?;
nacos.deregister("my-app", "192.168.1.10", 3000, "DEFAULT_GROUP")?;
```

## `boot` 模块 — 一键启动

```rust
pub struct Config {
    pub app_name: String,              // 默认：运行时二进制名
    pub host: String,                  // 默认 "0.0.0.0"
    pub port: u16,                     // 必填
    pub database_url: String,          // 必填
    pub redis_url: String,             // 必填
    pub redis_pool_size: usize,        // 默认 10
    pub advertise_ip: Option<String>,  // 缺省自动探测；探测失败启动报错
    pub extra: toml::Table,            // #[serde(flatten)]，未声明的键全落这里
}

impl Config {
    pub fn extra<T: DeserializeOwned>(&self) -> Result<T, HrError>;  // 解析业务自定义段
}

pub struct AppState {                  // hr 根导出，Clone
    pub config: Config,
    pub pg: sqlx::PgPool,
    pub redis: deadpool_redis::Pool,
    pub nacos: Nacos,
}

pub fn default_app_name() -> String;
pub fn detect_ip() -> Option<IpAddr>;

pub async fn run_env(routes: impl IntoIterator<Item = Router<AppState>>) -> Result<(), HrError>;
pub async fn run(config: Config, nacos: Nacos, routes: impl IntoIterator<Item = Router<AppState>>)
    -> Result<(), HrError>;
```

`run_env` 做完全套：`NACOS_*` 环境变量 → Nacos 配置中心读 `Config`（data_id 缺省为二进制名，group 取 `NACOS_GROUP`）→ 连 PostgreSQL / Redis → 合并路由 → 向 Nacos 注册服务 → 监听服务 → ctrl_c / SIGTERM 优雅停机并反注册。

想自己掌控参数（配置来源、Nacos 连接方式）时用 `run(config, nacos, routes)`：流程相同，跳过环境变量与配置中心读取。

### 业务模块的固定写法

每个业务模块暴露一个零参路由函数，`main` 一行启动：

```rust
// src/user.rs
pub fn routes() -> axum::Router<hr::AppState> {
    axum::Router::new().route("/user/{id}", axum::routing::get(get_user))
}

async fn get_user(State(st): State<hr::AppState>, Path(id): Path<i64>) -> hr::HrResult<User> {
    let user = find_user(&st.pg, id).await?;   // st.config / st.pg / st.redis / st.nacos
    hr::Hr::ok(user)
}

// src/main.rs
#[tokio::main]
async fn main() -> Result<(), hr::HrError> {
    hr::boot::run_env([user::routes(), order::routes()]).await
}
```

### 配置中心里的 TOML 示例

```toml
port = 3000
database_url = "postgres://user:pass@127.0.0.1:5432/mydb"
redis_url = "redis://127.0.0.1:6379"
advertise_ip = "192.168.1.10"   # 可省略，省略则自动探测

# 业务自定义键/段全部落进 config.extra，随取随解析：
[biz]
timeout = 30
```

```rust
#[derive(serde::Deserialize)]
struct Biz { timeout: u64 }

let biz: Biz = st.config.extra()?;
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

`examples/boot.rs`（需要真实的 Nacos + PostgreSQL + Redis）：

```bash
export NACOS_SERVER_ADDR=127.0.0.1:8848
# 配置中心放 data_id=boot（group DEFAULT_GROUP）的 TOML，见上文示例
cargo run --example boot
curl localhost:3000/health   # {"code":0,"message":"ok","data":{"app":"boot","redis":"PONG"}}
curl localhost:3000/user/1   # 查 PostgreSQL
```

## 给 AI 的提示（贴到你的服务项目里）

> 本项目所有 HTTP handler 必须使用 `hr` 库返回响应：成功返回 `Ok(Hr::ok(data))`（或 `Hr::page(list, total, page, page_size)` 分页），失败返回 `Err(...)`，handler 签名用 `HrResult<T>` 或 `Hr<T>`。不要手写 JSON 响应结构，不要改 HTTP 状态码。

> 服务接入：`main` 里用 `hr::boot::run_env([模块::routes(), ...]).await` 一行启动；每个业务模块暴露零参的 `routes() -> axum::Router<hr::AppState>`，handler 通过 `State(hr::AppState)` 拿 `config`/`pg`/`redis`/`nacos`。业务自定义配置写进配置中心 TOML，用 `state.config.extra::<T>()` 解析，不要另开配置文件。
## 测试

```bash
cargo test
```

17 个集成测试：0.1 的 9 个信封测试（成功/失败精确形状、`data` 省略、`HrResult` 两条路径、anyhow / StatusCode / &str 转换、分页形状），加上 `merge_all` 合并路由、二进制名缺省、`NACOS_SERVER_ADDR` 缺失报错、`Config` 默认值与 `extra` 解析、缺必填字段报错、坏 TOML 报 500、sqlx / redis / nacos / toml 错误转换。`cargo check --no-default-features` 验证纯信封模式可编译。
