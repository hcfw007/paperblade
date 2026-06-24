# 架构

PaperBlade 是一个本地优先的桌面 PDF 工具集。它用 SvelteKit SPA 搭配 Tauri
(Rust)外壳,真正的 PDF 处理交给打包内置的命令行引擎(qpdf、Ghostscript)。
文件全程不离开本机。

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
│  - 派生并监管引擎进程                           │
└───────────────┬───────────────────────────────┘
                │  std::process / sidecar
┌───────────────▼───────────────────────────────┐
│  CLI 引擎  (打包的 sidecar)                     │
│  qpdf  ·  ghostscript                          │
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
| `tauri.conf.json` | 窗口、打包目标、图标、CSP |
| `capabilities/default.json` | 主窗口的权限授予 |
| `Cargo.toml` | Rust 依赖(tauri、serde、插件) |

目前 `invoke_handler![]` 是空的 —— 还没有任何命令。第一步实质工作就是注册一个
命令并接通首个引擎调用(见 ROADMAP)。

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
    // 2. 用正确的参数派生 qpdf
    // 3. 把非零退出码 / stderr 映射成 Err(String)
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

qpdf 和 Ghostscript 作为 Tauri **sidecar** 随包发布(在 `bundle.externalBin`
中声明),因此从应用包内解析,而不依赖用户的 `PATH`。Rust 通过 shell/进程 API
派生它们,传入绝对路径,读取退出码和 stderr。

| 引擎 | 使用方 |
|------|--------|
| qpdf | 合并、拆分、加/解密、结构性操作 |
| Ghostscript | 压缩(降采样)、部分转换路径 |

水印和 PDF↔图片转换可能需要额外的渲染器(如 PDF 栅格化引擎);该引擎选型推迟到
排期这些工具时再定。

## 安全模型

- **无网络。** 核心功能零出站请求。
- **最小权限。** `capabilities/default.json` 只授予窗口所需的能力;按命令逐项、
  有意识地放宽。
- **CSP。** `tauri.conf.json` 当前设为 `csp: null`(开发便利)。发布构建前需收紧
  为真实策略。
- **边界校验。** 每个命令都在 Rust 侧重新校验路径/选项;前端校验只为体验,绝不
  作为信任边界。

## 构建与运行

```sh
pnpm install
pnpm tauri dev     # SvelteKit 开发服务 + Tauri 窗口
pnpm tauri build   # 生产打包
```

`pnpm dev` 只单独跑前端(浏览器内,无法调用 Rust 命令)—— 适合纯 UI 开发。
