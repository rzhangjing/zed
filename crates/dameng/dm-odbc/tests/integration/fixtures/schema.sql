-- L3 真机集成测试的建表脚本。
--
-- **必须幂等**：脚本先 `DROP TABLE IF EXISTS` 再 `CREATE TABLE`，所以连跑两次、或在
-- 上一次失败留下的残骸上跑，结果都一样（T3.7 的验收项之一）。
--
-- 语法约束（由 `common::apply_schema` 的分号切分决定）：
--   1. 每条语句独立一行块，用 `;` 结尾；
--   2. 字符串字面量里**不得**出现分号；
--   3. 以 `--` 开头的整行是注释，切分时会丢掉。
--
-- 列类型的选择全部来自真机实测（§7 的「真机实测补注（M3/T3.5）」）：
--   * 没有 `BOOLEAN` —— 达梦报 `22019 / -6119 无效的数据类型`，布尔语义用 `BIT`；
--   * `DOUBLE` 是双精度，`REAL` 才是单精度（达梦的 `FLOAT` 也是双精度，这里用 `DOUBLE` 更直白）；
--   * `BINARY(8)` 是定长二进制，`VARBINARY(300)` 是变长，两者走同一条 `SQL_C_BINARY` 路径但
--     定长列会被服务端补零，正好用来钉住这个差异。

DROP TABLE IF EXISTS DM_T_TYPES;

CREATE TABLE DM_T_TYPES (
    ID INT NOT NULL PRIMARY KEY,
    C_CHAR CHAR(10),
    C_VARCHAR VARCHAR(100),
    C_CLOB CLOB,
    C_TINYINT TINYINT,
    C_SMALLINT SMALLINT,
    C_INT INT,
    C_BIGINT BIGINT,
    C_DECIMAL DECIMAL(20,4),
    C_NUMERIC NUMERIC(10,2),
    C_REAL REAL,
    C_DOUBLE DOUBLE,
    C_BIT BIT,
    C_DATE DATE,
    C_TIME TIME,
    C_TIMESTAMP TIMESTAMP,
    C_VARBINARY VARBINARY(300),
    C_BINARY BINARY(8),
    C_BLOB BLOB
);

-- 字符集 / 参数绑定用的小表：一列文本、一列二进制、一列整数。
DROP TABLE IF EXISTS DM_T_STR;

CREATE TABLE DM_T_STR (
    ID INT NOT NULL PRIMARY KEY,
    S VARCHAR(200),
    B VARBINARY(200),
    N INT
);

-- 大对象用例：只放两列 LOB，这样插入语句的参数个数不会盖过用例本身。
DROP TABLE IF EXISTS DM_T_LOB;

CREATE TABLE DM_T_LOB (
    ID INT NOT NULL PRIMARY KEY,
    C CLOB,
    B BLOB
);

-- 事务用例（begin/commit/rollback 与 autocommit 的交互）。
DROP TABLE IF EXISTS DM_T_TXN;

CREATE TABLE DM_T_TXN (
    ID INT NOT NULL PRIMARY KEY,
    V INT
);

-- 错误码采集用（§10.3 `errors.rs`）：主键冲突、NOT NULL、CHECK、UNIQUE 各有落点。
DROP TABLE IF EXISTS DM_T_ERR;

CREATE TABLE DM_T_ERR (
    ID INT NOT NULL PRIMARY KEY,
    N INT NOT NULL,
    U VARCHAR(20) UNIQUE,
    C INT CHECK (C > 0)
);
