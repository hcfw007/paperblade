# 架构

PaperBlade 是一个本地优先的桌面 PDF 工具集。它用 SvelteKit SPA 搭配 Tauri
(Rust)外壳。结构性 PDF 操作由内嵌的纯 Rust 库(lopdf)在命令里直接完成,重渲染
类操作(压缩、栅格化)再交给打包内置的 CLI 引擎(Ghostscript)。文件全程不离开
本机。

## 目标与约束

- **纯本地。** 所有处理都在设备上完成,核心功能不发任何网络请求。
- **小体积。** 选 Tauri 而非 Electron;打包 CLI 引擎,而不是堆一堆 PDF 库。
- **macOS 优先。** 引擎打包方案稳定后再扩展到其他平台。
- **一套模式,多个工具。** 每个工具都复用同一条 前端 → 命令 → 引擎 路径。

## 分层

```
┌─────────────────────────────────────────────┐
│  SvelteKit SPA  (src/)                        │
│  routes/  ·  lib/  ·  styles/                 │
│  - 工具页面、文件选择、进度 UI                  │
└───────────────┬───────────────────────────────┘
                │  @tauri-apps/api  invoke(cmd, args)
┌───────────────▼───────────────────────────────┐
│  Tauri 外壳  (src-tauri/src/)                  │
│  - #[tauri::command] 处理器                    │
│  - 输入校验、路径处理                           │
│  - lopdf 直接处理结构性操作                     │
│  - 重活则派生并监管引擎进程                     │
└───────────────┬───────────────────────────────┘
                │  std::process / sidecar(仅重活)
┌───────────────▼───────────────────────────────┐
│  CLI 引擎  (打包的 sidecar,按需)               │
│  ghostscript ·(渲染器待定)                     │
└────────────────────────────────────────────────┘
```

前端从不直接碰文件系统或引擎。它只收集用户意图(文件、选项)并调用 Rust
命令,所有副作用都由 Rust 掌管。

## 前端(`src/`)

SvelteKit 5(runes)运行在 SPA 模式:`ssr = false`,`adapter-static` 配
`index.html` 回退。没有服务端;构建产物直接在 Tauri 窗口内加载。

| 路径 | 职责 |
|------|------|
| `routes/+layout.svelte` | 应用外壳:顶栏、品牌标识、返回工具列表链接 |
| `routes/+page.svelte` | 首页 —— Hero + 由 `TOOLS` 驱动的工具网格 |
| `routes/<tool>/+page.svelte` | 每个工具一个页面(选文件、设选项、执行) |
| `lib/tools.ts` | 工具目录的唯一数据源 |
| `styles/tokens.css` | 设计 token(颜色、间距、字号、动效) |
| `styles/global.css` | 重置样式 + 消费 token 的元素默认样式 |

往网格里加工具是数据驱动的:在 `TOOLS` 追加一条记录,等其页面可用后再把
`available` 置为 `true`。首页把可用工具渲染成链接,其余渲染成禁用的
"Soon" 卡片。

## Tauri 外壳(`src-tauri/`)

| 路径 | 职责 |
|------|------|
| `src/main.rs` | 二进制入口 → 调用 `paperblade_lib::run()` |
| `src/lib.rs` | Builder 配置 + `invoke_handler![]` 命令注册表 |
| `src/pdf.rs` | 各工具共用:路径校验、文件名标签、页数、`/Type` 判断 |
| `src/merge.rs` | 合并算法(对象重编号 + 页树重建) |
| `src/split.rs` | 拆分算法(页码范围解析 + 逐片删页) |
| `src/encrypt.rs` | 加解密(AES-128 标准安全处理器) |
| `src/compress.rs` | 压缩(构建 Ghostscript 参数 + 派生进程 + 读结果) |
| `tauri.conf.json` | 窗口、打包目标、图标、CSP |
| `capabilities/default.json` | 主窗口的权限授予 |
| `Cargo.toml` | Rust 依赖(tauri、serde、插件) |

`invoke_handler![]` 目前注册了 `merge_pdfs`、`split_pdf`、`page_count`、
`encrypt_pdf`、`decrypt_pdf` 和 `is_encrypted`。命令本身
只做参数搬运,真正的 PDF 逻辑住在各自的模块里(`merge.rs` / `split.rs`),路径校验
和页树探查等共用小工具住在 `pdf.rs`。

## 前端 ↔ Rust 契约

每个工具对应一个命令。保持结构统一,这样 UI 层足够薄,每个工具都走同一套流程。

```ts
// 前端调用(示意)
import { invoke } from "@tauri-apps/api/core";

const output: string = await invoke("merge_pdfs", {
  inputs: ["/abs/a.pdf", "/abs/b.pdf"],
  output: "/abs/merged.pdf",
});
```

```rust
// Rust 端(示意)
#[tauri::command]
async fn merge_pdfs(inputs: Vec<String>, output: String) -> Result<String, String> {
    // 1. 校验路径存在且为 PDF
    // 2. 用 lopdf 加载、重映射对象、合并页树(或派生引擎处理重活)
    // 3. 把库/引擎错误映射成 Err(String)
    // 4. 成功则返回输出路径
}
```

约定:

- **错误统一用 `Result<T, String>`。** 返回可直接展示给用户的信息;详细上下文在
  Rust 侧记录日志。命令里绝不 panic。
- **路径来自系统对话框**,是绝对路径,并在使用前于 Rust 侧校验 —— 前端输入一律
  视为不可信。
- **长任务发进度事件**(`window.emit`)而非阻塞 UI;页面订阅后渲染进度条。

## 引擎集成

分两层:

- **lopdf(内嵌 Rust 库)** 处理结构性操作 —— 合并、拆分、加/解密。命令里直接
  调用,无外部进程、无 dylib、无 `PATH` 依赖。`src-tauri/src/merge.rs` 是首个范例。

  拆分走的是"克隆再删页"而不是"从零搭页树":每个切片都是整份文档的副本,删掉
  区间外的页后再 `prune_objects()` 回收孤儿对象。这样被保留页引用的字体、图片、
  注释都会自然跟着走。代价是嵌入字体在每个切片里各留一份,切片体积之和会大于
  原文件 —— 正确性优先,后续可在压缩工具里做字体子集化。
- **CLI 引擎(按需 sidecar)** 处理 lopdf 力所不及的重活。Ghostscript 用于压缩
  (降采样);Rust 通过进程 API 派生,传绝对路径,读退出码 + stderr。

  `compress.rs` 把"构建参数"和"跑进程"分开:`gs_args()` 是纯函数,所以参数
  组合(尤其是 `-dSAFER` 有没有掉)在没有 gs 的机器上也能测 —— CI 就是这样。

  二进制的解析顺序是"先应用包内、再回退 `PATH`"。**目前包内那一份还不存在**:
  brew 装的 gs 链接了十几个 homebrew dylib,不能原样分发,静态化和
  `bundle.externalBin` 声明都是 M3 的活。所以现在压缩依赖用户自己装 gs,
  `has_compression_engine` 命令让页面能提前说清楚,而不是等用户点了才报错。

  加密这条路上有个不直观的地方:`Document::load` 碰到加密文件时,一旦空密码
  认证失败就直接返回,连一个对象都不读。所以解密必须走
  `load_with_password`,而不是"先 load 再 decrypt"。

| 操作 | 引擎 |
|------|------|
| 合并、拆分、加/解密 | lopdf(库) |
| 压缩(降采样) | Ghostscript(sidecar) |
| 水印、PDF↔图片转换 | 渲染器待定(排期时再选) |

引入第一个 sidecar 前不需要任何二进制打包;它推迟到压缩工具排期时再做。

## 安全模型

- **无网络。** 核心功能零出站请求。
- **最小权限。** `capabilities/default.json` 只授予窗口所需的能力;按命令逐项、
  有意识地放宽。
- **CSP。** `tauri.conf.json` 当前设为 `csp: null`(开发便利)。发布构建前需收紧
  为真实策略。
- **边界校验。** 每个命令都在 Rust 侧重新校验路径/选项;前端校验只为体验,绝不
  作为信任边界。
- **密码只过内存。** 加解密的密码作为命令参数传入,用完即随栈帧释放:不落盘、
  不写日志、不进错误信息。前端在操作成功后清空输入框。解密失败一律回
  "Wrong password.",不透传库的内部错误 —— 那些信息只会帮到猜密码的人。

## 构建与运行

```sh
pnpm install
pnpm tauri dev     # SvelteKit 开发服务 + Tauri 窗口
pnpm tauri build   # 生产打包
```

`pnpm dev` 只单独跑前端(浏览器内,无法调用 Rust 命令)—— 适合纯 UI 开发。
