-- 示例迁移：hr 仓库自带，examples/boot.rs 与集成测试编译期内嵌。
CREATE TABLE IF NOT EXISTS hr_demo (
    id BIGSERIAL PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
