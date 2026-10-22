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

## 0.7 模块（两态：boot 全家桶 / 纯信封）

| feature | 内容 | 依赖 |
|---|---|---|
| `boot`（默认） | 响应信封 + 一键启动：配置 → 连接 → 迁移 → 注册 → 服务 → 停机反注册 | sqlx 0.9 / deadpool-redis 0.23 / nacos-sdk 0.3 / toml / tokio / tracing |

pg/redis/nacos 不再单独开 feature，连接函数也不公开：**连库只有 `hr::boot::serve` 启动时做一次**，业务代码只写 `[pg]`/`[redis]` 配置段、从 `AppState` 取连接池。只要响应信封？关掉默认 feature：`default-features = false`。
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
| `From<...> for HrError` | `StatusCode` / `anyhow::Error` / `String` / `&str` 自动转换（基础设施错误统一 `internal error`，原文进日志） |
| `hr::merge_all([...])` | 合并各业务模块路由；框架错误（404/405/400 等 ≥400）保留真实状态码，body 统一包装成信封 |
| `pg::PgConfig` / `redis::RedisConfig` / `sqlx::migrate!` | `[pg]`/`[redis]` 配置段（连接由 boot 启动时执行）/ 编译期迁移 |
| `nacos::Nacos` | 配置中心读取 + 服务注册/注销 |
| `boot::serve` / `boot::serve_with` + `with_migrator` / `with_hook` | 一键启动整套服务，链式挂迁移与启动钩子（见下文） |
| `AppState`（boot） | 全家桶状态：config / pg / redis / nacos |
---

## `Hr<T>` — 响应信封

结构体字段是私有的，只能通过构造器创建，保证所有响应结构一致。`Hr<T>` / `HrPage<T>` 同时实现了 `Serialize` 和 `Deserialize`，服务间互调（BFF 调 hr 服务）可直接把响应解析回来。

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

`data` 不出现在响应里。HTTP 状态仍是 200。`code = 0` 表示成功，`err(0, ..)` 会被当作 500 处理。例外：框架错误（404 无路由、405 方法不对、400 路径参数错等 ≥400 响应）由 `merge_all` 保留真实 HTTP 状态码，body 仍是信封（code = 状态码，message 取 axum 原文或规范原因短语）。

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
| `anyhow::Error` | `{"code":500,"message":"internal error"}`（原文 `warn!` 进日志） |
| `axum::http::StatusCode` | `{"code":<状态码数字>,"message":"<标准原因短语>"}` |

| `sqlx::Error` / `sqlx::migrate::MigrateError`（pg feature） | `{"code":500,"message":"internal error"}`（原文进日志） |
| `deadpool_redis::PoolError` / `CreatePoolError` / `redis::RedisError`（redis feature） | 同上 |
| `nacos_sdk::api::error::Error`（nacos feature） | 同上 |
| `toml::de::Error`（nacos feature） | 同上 |
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
pub use sqlx;             // 重新导出，业务代码无需直接依赖 sqlx
pub use sqlx::migrate;    // 迁移：hr::pg::migrate::Migrator + sqlx::migrate! 宏

pub struct PgConfig {     // TOML [pg] 段，全默认可整段省略
    pub max_connections: u32,     // 默认 10
    pub acquire_timeout_secs: u64, // 默认 5
}

// 连接函数为 crate 内部，boot 启动时调用；业务代码从 AppState 拿池
```

```rust
async fn user(State(st): State<hr::AppState>) -> hr::HrResult<User> {
    let name: Option<(String,)> = sqlx::query_as::<_, (String,)>("select name from t_user where id = $1")
        .bind(1)
        .fetch_optional(&st.pg)          // 连接池在 AppState 里
        .await
        .map_err(hr::HrError::from)?;   // sqlx::Error → {"code":500,"message":"internal error"}


### 数据库迁移（编译期内嵌，部署零目录）

业务仓库建 `migrations/` 目录（sqlx 命名 `<序号>_<描述>.sql`，如 `0001_create_user.sql`），`main` 里用 `with_migrator(sqlx::migrate!("./migrations"))` 挂上，启动时自动执行（`_sqlx_migrations` 记账、幂等）：

```rust
use hr::pg::sqlx;   // 宏用，无需在 Cargo.toml 加 sqlx 依赖

hr::boot::serve([user::routes()]).with_migrator(sqlx::migrate!("./migrations")).await
```

不调用 `with_migrator` 就不跑迁移。约定：序号递增；已发布到生产的迁移文件不改内容（sqlx 按 checksum 校验）；只支持上行迁移。

## `redis` 模块 — Redis 连接池

```rust
pub use deadpool_redis;   // 重新导出

pub struct RedisConfig {      // TOML [redis] 段，全默认可整段省略
    pub pool_size: usize,             // 默认 10
    pub wait_timeout_secs: Option<u64>, // 默认 None（库默认，不设等待超时）
}

// 连接函数为 crate 内部，boot 启动时调用；业务代码从 AppState 拿池
```

```rust
let mut conn = st.redis.get().await.map_err(hr::HrError::from)?;
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
| `NACOS_DATA_ID` | | 运行时二进制名 | `boot::serve` 读配置用；二进制改名部署时显式覆盖 |
| `NACOS_GROUP` | | `DEFAULT_GROUP` | 读配置用；服务注册分组走 `Config.group` |

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
    pub group: String,                 // Nacos 注册分组，默认 "DEFAULT_GROUP"
    pub port: u16,                     // 必填
    pub database_url: String,          // 必填
    pub redis_url: String,             // 必填
    pub pg: hr::pg::PgConfig,          // [pg] 段，省略 = 全默认
    pub redis: hr::redis::RedisConfig, // [redis] 段，省略 = 全默认（0.3 的平铺 redis_pool_size 改到这里）
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

pub fn serve(routes: impl IntoIterator<Item = Router<AppState>>) -> Boot;
pub fn serve_with(config: Config, nacos: Nacos, routes: impl IntoIterator<Item = Router<AppState>>) -> Boot;

impl Boot {                                        // Boot: IntoFuture，链式配好后 .await 即启动
    pub fn with_migrator(self, migrator: sqlx::migrate::Migrator) -> Self;
    pub fn with_hook<F, Fut>(self, hook: F) -> Self
    where
        F: FnOnce(AppState) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), HrError>> + Send + 'static;
}

`serve` 做完全套：`NACOS_*` 环境变量 → Nacos 配置中心读 `Config`（data_id 缺省为二进制名，group 取 `NACOS_GROUP`）→ 连 PostgreSQL / Redis → 迁移（`with_migrator` 挂了才跑）→ 逐个执行启动钩子（`with_hook`，见下文）→ 合并路由 → 监听 → 注册 Nacos（**先监听后注册**：绑定失败不会把坏实例挂上网关）→ ctrl_c / SIGTERM 优雅停机并反注册。服务注册分组取 `Config` 的 `group` 键（默认 `DEFAULT_GROUP`）。

想自己掌控参数（配置来源、Nacos 连接方式）时用 `serve_with(config, nacos, routes)`：流程相同，跳过环境变量与配置中心读取。


注意：handler 内 panic 会让该请求连接直接断开（拿不到信封）。需要兜底时在 Router 上加 `tower-http` 的 `CatchPanicLayer`（业务仓库自行引入，本库不强制这个依赖）。
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
    hr::boot::serve([user::routes(), order::routes()])
        .with_migrator(sqlx::migrate!("./migrations"))
        .await
}
```

### 启动钩子

一次性前置步骤（播种、预热）与常驻后台任务，都写成**普通异步函数**，用 `with_hook` 逐个链式追加（不同函数的类型各不相同，无法装进同一个数组）：

```rust
async fn warm_up(st: hr::AppState) -> Result<(), hr::HrError> {
    // 前置步骤：用 st.pg / st.redis 播种、预热；返回 Err 则整个启动失败
    Ok(())
}

async fn start_flusher(st: hr::AppState) -> Result<(), hr::HrError> {
    tokio::spawn(flush_loop(st.redis));   // 常驻任务：裸 tokio::spawn，随进程退出
    Ok(())
}

hr::boot::serve([user::routes()])
    .with_migrator(sqlx::migrate!("./migrations"))
    .with_hook(warm_up)          // 按调用顺序执行
    .with_hook(start_flusher)
    .await
```

- 钩子在连接与迁移之后、Nacos 注册与监听之前执行；任一返回 `Err` 即整体启动失败（不注册、不监听）。
- 钩子收到 `AppState` 的 Clone（字段全是引用计数句柄，克隆近零成本）；钩子之间不共享修改。
- 钩子里 `tokio::spawn` 的任务与 serve 并行，进程退出即结束。

### 配置中心里的 TOML 示例

```toml
port = 3000
database_url = "postgres://user:pass@127.0.0.1:5432/mydb"
redis_url = "redis://127.0.0.1:6379"
group = "DEFAULT_GROUP"         # 服务注册分组，可省略
advertise_ip = "192.168.1.10"   # 可省略，省略则自动探测

[pg]                    # 整段可省略 = 全默认
max_connections = 20
acquire_timeout_secs = 10

[redis]                 # 整段可省略 = 全默认
pool_size = 32
# wait_timeout_secs = 8

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

> 服务接入：`main` 里用 `hr::boot::serve([模块::routes(), ...]).with_migrator(sqlx::migrate!("./migrations")).await` 启动（不用迁移就不调 `with_migrator`）；启动钩子是收 `hr::AppState` 的普通异步函数，`with_hook` 逐个追加，返回 Err 则不启动；基础设施错误（数据库/Redis/Nacos/TOML）经 `?` 转换后响应体固定为 `{"code":500,"message":"internal error"}`（原文在服务日志），不要把内部错误细节拼进响应；每个业务模块暴露零参的 `routes() -> axum::Router<hr::AppState>`，handler 通过 `State(hr::AppState)` 拿 `config`/`pg`/`redis`/`nacos`。连接参数写配置中心 `[pg]`/`[redis]` 段（整段可省略 = 全默认）；连接由 `hr::boot` 启动时完成，业务代码不自行连库（连接函数不公开），池从 `AppState.pg`/`AppState.redis` 取；业务自定义配置用 `state.config.extra::<T>()` 解析，不要另开配置文件。

## 日志

hr 通过 [`tracing`](https://docs.rs/tracing) 门面输出日志，业务侧初始化 subscriber 即可看到：

```rust
fn main() {
    tracing_subscriber::fmt::init();   // 用 RUST_LOG 控制级别，如 RUST_LOG=info
    // ...
}
```

| 等级 | 内容 |
|---|---|
| `error` | 启动失败（连库 / 注册 / 监听）、停机注销失败 |
| `warn` | Nacos 配置不存在（`load_config` 返回 `None` 前） |
| `info` | 连接池就绪、Nacos 连接、服务注册 / 注销、开始监听、收到停机信号 |
| `debug` | `NACOS_*` 环境变量解析结果、配置读取请求 |
| `trace` | TOML 配置解析 |

密码类字段（`database_url` / `redis_url` / `NACOS_PASSWORD`）永不写入日志。

## 测试

```bash
cargo test
```

29 个集成测试：0.1 的 9 个信封测试（成功/失败精确形状、`data` 省略、`HrResult` 两条路径、anyhow / StatusCode / &str 转换、分页形状），加上 `merge_all` 合并路由、二进制名缺省、`NACOS_SERVER_ADDR` 缺失报错、`Config` 默认值与 `extra` 解析（含 `[pg]`/`[redis]` 配置段覆盖、旧平铺键落 `extra`）、缺必填字段报错、坏 TOML 报 500、sqlx / redis / nacos / toml / migrate 错误转换、`migrate!` 宏内嵌校验，加上框架错误信封（404/405 真实状态码 + `{"code":404,"message":"Not Found"}`）与 body 非空豁免透传（原状态码、原 content-type）、`err(0)`→500、`Hr`/`HrPage` 反序列化往返、`group` 配置默认与覆盖、基础设施错误掩码为 `internal error`。`cargo check --no-default-features` 验证纯信封模式可编译。
