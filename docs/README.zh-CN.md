# yolow64

[English](../README.md)

面向 Rust 的 Base64 编解码库，提供简洁 API 和运行时分派的 SIMD 加速。运行时只依赖 Rust 标准库。

## 故事

在研发 [PageSift](https://github.com/YOLOW92/pagesift) 时，我们需要为浏览器截图、MCP 资源和代理认证处理 Base64。这些场景让正确性、分配控制和实际吞吐量同样重要。yolow64 将这套编解码器独立出来，采用 MIT 许可。

## 概览

- 支持标准和 URL-safe 字母表，以及带填充和无填充格式。
- 提供切片、追加、拥有型输出、校验、流式解码和 data URL API。
- 可选 `parallel` feature，按输入尺寸和缓存自动选择 worker 数。
- 支持的 x86-64 CPU 使用 AVX2 批量路径和 BMI2 短解码路径，两者独立检测；支持的 AArch64 CPU 使用 NEON；否则回退标量路径。
- 需要 Rust 1.89 或更新版本和 `std`；**运行时无第三方依赖**。
- 下方提供实测数据及可运行基准；短输入并非全部领先。

## 工作原理

![解码流水线](figures/decode-pipeline.svg)

*AVX2 批量路径与短输入精确长度 / BMI2 `pext` 路径。*

![CPU 分派与回退](figures/cpu-dispatch.svg)

*AVX2 与 BMI2 独立检测；只有两者都存在时才执行 `pext` 短路径。*

## 发布说明

0.2.x 的性能、兼容性和安全改动见 [`CHANGELOG.md`](../CHANGELOG.md)；完整实验记录（包括被否决的内核）见 [`research/`](../research/README.md)。

## 安装

已发布到 crates.io：

```toml
[dependencies]
yolow64 = "0.2"
```

## 用法

```rust
use yolow64::{Alphabet, Codec, Decoder, STANDARD};

fn main() -> Result<(), yolow64::Error> {
    let encoded = STANDARD.encode_to_string(b"hello")?;
    assert_eq!(encoded, "aGVsbG8=");
    assert_eq!(STANDARD.decode_to_vec(encoded.as_bytes())?, b"hello");

    let url = Codec {
        alphabet: Alphabet::UrlSafe,
        padding: false,
        ignore_whitespace: false,
    };
    assert_eq!(url.encode_to_string(&[0xfb, 0xff])?, "-_8");

    let mut decoder = Decoder::new(STANDARD);
    let mut output = Vec::new();
    decoder.feed(b"aGV", &mut output)?;
    decoder.feed(b"sbG8=", &mut output)?;
    decoder.finish(&mut output)?;
    assert_eq!(output, b"hello");
    Ok(())
}
```

也可以运行 `cargo run --example basic`。

`STANDARD` 使用标准字母表、在需要时要求填充，并拒绝空白。设置 `ignore_whitespace: true` 后，**仅**忽略空格、制表符、CR 和 LF，不会跳过任意 MIME 杂字符。`padding: false` 时禁止填充；两种模式都拒绝非零的末尾未使用位。

预分配缓冲区可使用 `Codec::encoded_len` 计算编码容量，`Codec::encode` 和 `Codec::decode` 返回写入字节数，`Codec::validate` 返回精确解码长度。`append_encode` 和 `append_decode` 向 `Vec<u8>` 追加；`encode_data_url` 不校验 MIME 类型。解码出错时，调用方提供的缓冲区可能留下部分输出，应丢弃；流式 `Decoder` 出错前也可能已追加输出，如果需要整条消息有效，应丢弃这些输出。`InvalidByte` 包含原始输入中的字节和偏移；其他错误包括 `InvalidLength`、`InvalidPadding` 和 `OutputTooSmall`。

## 性能

2026-11-11 在同一台机器、同一输入和同一进程中对 8 个实现做了完整对比。完整表格、原始数据和每个库的精确配置见 [`benchmarks/README.md`](../benchmarks/README.md)；包含被否决内核的实验记录见 [`research/experiments.md`](../research/experiments.md)；计数器、缓存未命中与交叉点分析见 [`research/encode-counters.md`](../research/encode-counters.md)，22 个实现的 encode 专项横评见 [`research/encode-comparison.md`](../research/encode-comparison.md)；decode 状态、cold-buffer 数据和已否决的内核见 [`research/decode.md`](../research/decode.md)。在更新前的 22 库横评中，yolow64 的短输入排第 3，192 B–4 KiB 排第 2，8 KiB–16 MiB 排第 3；1 MiB 排第 1。加入 8/16/32 字节 fast path 后，它在 8 库测试中于 8、12、16、24、32、48、96 B 最快。计数器报告补充了每次调用的指令数、D1/LLd 未命中率和交叉点。

环境：Intel Core i7-10700（Comet Lake）、AVX2/BMI2、无 AVX-512、Linux x86-64、Rust 1.99.0、release fat LTO。机器是共享的，`schedutil` 在运行；8–128 B 和 64 KiB 较稳定，1–4 MiB 的差异噪声较大。外部库列共运行 3 个独立进程，每个进程 7 轮并轮换顺序；表中为 3 个进程中位数的中位数。`yolow64` 解码列为 6 个独立进程的中位数。

### 短输入，纳秒/次（越低越好）

| 输入 | yolow64 编码 | 最快编码 | yolow64 解码 | 最快解码 |
|---:|---:|---:|---:|---:|
| 8 B | 6.02 | 6.02（yolow64） | 14.26 | 7.90（Turbo-Base64） |
| 12 B | 7.35 | 7.35（yolow64） | **7.00** | **7.00（yolow64）** |
| 16 B | 6.20 | 6.20（yolow64） | **8.53** | **8.53（yolow64）** |
| 24 B | 9.01 | 9.01（yolow64） | **10.14** | **10.14（yolow64）** |
| 32 B | 7.29 | 7.29（yolow64） | 19.14 | 10.36（base64-simd） |
| 48 B | 9.95 | 9.95（yolow64） | **12.20** | **12.20（yolow64）** |
| 64 B | 13.41 | 12.11（Turbo-Base64） | 23.18 | 12.81（Turbo-Base64） |
| 96 B | 11.43 | 11.43（yolow64） | **15.68** | **15.68（yolow64）** |
| 128 B | 17.40 | 13.74（Turbo-Base64） | **17.18** | **17.18（yolow64）** |

短表 `最快解码` 列来自默认部分校验的录制结果；在 `FULLCHECK=1` 下 Turbo-Base64 会更慢，因此 yolow64 的全量校验胜出不受影响。

![短输入解码延迟 — 全部对比库](figures/short-all.svg)

*8 个对比库的短输入解码延迟，纳秒/次；越低越好。*

短输入编码现在是强项：8、12、16、24、32、48、96 B 由 yolow64 最快；Turbo-Base64 在 64 B 和 128 B 领先。短输入解码现在在 12、16、24、48、96、128 B 最快。精确长度 decode fast path 会绕过通用 dispatch，处理无 padding 的 16/32/64/128 字符输入；24 字符（16 B）现在使用单个 `avx2+bmi2` 函数，44/172 字符（32/128 B）使用 `pext` 尾部。22 库横评中 `tape-base64` 在 24 B 编码仍略快（8.78 vs 9.01）。新的 8/16/32 字节编码单 load 路径复用 24 字节转换，只覆盖 padding 字符，省掉通用尾部。8 KiB–1 MiB 的 decode 通过 speculative validation 和 hash loop 提升 5–7%，详见 [`research/decode.md`](../research/decode.md)。

### 64 KiB 吞吐，GB/s（越高越好）

| 操作 | yolow64 | base64-scalar | base64-simd-engine | base64-simd | base64-turbo | encodify | simdutf | Turbo-Base64 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 编码 | 22.5 | 2.5 | 10.0 | 10.7 | 20.9 | 21.4 | 18.7 | **23.7** |
| 解码 | **20.1** | 2.1 | 12.3 | 7.9 | 18.0 | 18.5 | 14.5 | 19.4 |

### 大输入吞吐，GB/s（越高越好）

| 输入 | yolow64 编码 | Turbo-Base64 编码 | yolow64 解码 | Turbo-Base64 解码（full） |
|---:|---:|---:|---:|---:|
| 1 MiB | 22.3 | 21.0 | 20.2 | 16.7 |
| 4 MiB | 17.7 | 16.8 | 17.1 | 13.9 |

编码在 1 MiB 和 4 MiB 领先 Turbo-Base64，256 KiB 接近，在 4–64 KiB 仍落后约 5–8%。64 KiB 解码行使用 Turbo-Base64 `FULLCHECK=1`；其默认部分校验构建报告 26.1 GB/s。解码通过 speculative validation 和 hash loop 在 8 KiB–1 MiB 提升约 5–7%，现在 1 MiB 为 20.2 GB/s、4 MiB 为 17.1 GB/s。它对所有 block 做完整校验，在全量校验对比中从 64 KiB 到 4 MiB 领先；Turbo-Base64 使用 `FULLCHECK=1` 做同口径对比；其默认构建在 256 字节热循环中只检查 4 个 block 中的第 1 个（25%），`FULLCHECK=1` 才检查全部 block。4 MiB 两者差异在运行噪声内，不应把该尺寸当成稳定排名。

![解码吞吐 — 全部对比库](figures/decode-all.svg)

*8 个对比库的解码吞吐，GB/s；越高越好。Turbo 使用 FULLCHECK=1。*

![编码吞吐 — 22 库对比记录](figures/encode-all.svg)

*记录的 22 库编码对比，GB/s；越高越好。外部列未重新排名。*

在这轮 8 库测试之后，编码新增了自适应流式写入路径（当输入加输出超过检测到的末级缓存时使用非临时存储），并把每 192 字节循环的预取覆盖从 1 条 cache line 提升到 3 条。三进程定向测试得到 **1 MiB 22.4 GB/s、4 MiB 21.7 GB/s、16 MiB 13.0 GB/s**（多轮测试为 13.0–13.3 GB/s），16 MiB 高于原来的 7.36 GB/s（1.77x），也高于记录中的 `tape-base64` 10.96 GB/s 和 `base64-turbo` 10.70 GB/s。阈值是当前 CPU LLC 的 `3/7`，会随处理器变化；缓存信息未知或输出未对齐时回退到普通写入。详见 [`research/encode-counters.md`](../research/encode-counters.md)。

### CRLF 宽容解码，GB/s（越高越好）

| 输入 | yolow64（原生） | encodify（原生） | simdutf（原生） | Turbo-Base64（过滤+严格） |
|---:|---:|---:|---:|---:|
| 4 KiB | 7.3 | **9.9** | 7.9 | 1.2 |
| 64 KiB | 9.2 | **10.3** | 8.1 | 1.1 |
| 1 MiB | 8.5 | **9.7** | 7.8 | 1.1 |
| 4 MiB | 6.2 | **7.3** | 6.2 | 1.0 |

76 列 MIME 快速路径补上了大部分 CRLF 差距：旧实现 1 MiB 约比 encodify 慢 2 倍，现在约慢 10%。各库接受的输入和错误语义不同；`过滤+严格` 包含空白过滤时间。

### 拥有型、追加、流式和并行 API

- `encode_to_string`、`append_encode`、`decode_to_vec`、`append_decode` 直接写入 `Vec` 的空闲容量，不再先清零。
- `Decoder::feed` 对完整 quartet 走批量解码，只有 padding 或空白才退回逐字节路径。1 MiB 严格输入按 4 KiB 分块喂入，从旧实现的约 0.14 GB/s 提升到约 9 GB/s。
- `encode_streaming` 是一次性编码入口。4 KiB 起在输出对齐时使用 16 字节非临时存储，避免为输出做 read-for-ownership；cold-buffer 测试中 4 KiB–4 MiB 比缓存友好的 `encode` 快 1.39–1.68 倍。默认仍用 `encode`，适合复用或立即回读的输出缓冲区。
- `encode_to_writer` 用固定 64 KiB 缓冲区编码，峰值库内存不随输出增长。256 MiB 输入实测峰值 RSS 约 258 MiB，而 `encode_to_string` 约 599 MiB。
- 可选 `parallel` feature 提供缓存感知的 `encode_parallel` / `decode_parallel`。在这台 i7-10700 上，4 个 worker 在 4–6.5 MiB 提升约 20–25%；7 MiB 以上单线程流式写入更快，自动策略保持单线程。decode 暂时保留保守的 2 worker 策略，等它自己的交叉点测试。详见 [`benchmarks/README.md`](../benchmarks/README.md)、[`research/encode-counters.md`](../research/encode-counters.md) 和 [`docs/API.md`](API.md)。

## 功能与保证

- 调用方提供缓冲区的编解码 API 无需分配；拥有型和追加 API 根据需要分配。
- 运行时检测结果会缓存。AVX2 和 BMI2 独立检测；BMI2 `pext` 短解码尾部只在两者都存在时执行，只有 AVX2 或纯标量的 CPU 会回退且不会执行 `pext`。短输入严格解码还会对无 padding 的 16/32/64/128 字符输入使用精确长度 SIMD fast path。Linux x86-64 AVX2 解码器使用汇编批处理循环；其他受支持目标使用可用的 SIMD 或标量路径。可选并行功能由 feature 控制，默认 API 不会创建线程。
- 差分测试对照 `base64` 覆盖四种字母表/填充组合，另含边界、错误输入、空白、流式和缓冲区测试。

运行 `cargo test --release --locked` 测试库；参见 [API 摘要（英文）](API.md)，或运行 `cargo doc --no-deps` 生成 API 文档。

## 许可证

[MIT](../LICENSE)。第三方归属及许可声明见 [THIRD-PARTY.md](../THIRD-PARTY.md)。
