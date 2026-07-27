# 路线图

现状快照,以及通往可用 v0.1 的计划路径。各部分如何衔接见
[ARCHITECTURE.md](./ARCHITECTURE.md)。

## 引擎策略

结构性操作(合并、拆分、加/解密)用纯 Rust 库 **lopdf** 内嵌实现,无外部依赖;
重渲染类操作(压缩、栅格化)交给 CLI 引擎 **Ghostscript**,以独立进程调用。

## 当前进度

四个工具都已端到端打通:命令 + dialog 选文件/选目录 + 各自的页面,Rust 侧有
算法或参数构建的测试。水印和转换仍是脚手架。

压缩是唯一依赖外部引擎的工具,而那个引擎**还没有被打包进来** —— 当前是回退到
用户 `PATH` 上的 `gs`,页面在找不到时会直说。真正的 sidecar 打包见 M3 前置项。

| 工具 | 页面 | 命令 | 引擎 | 状态 |
|------|------|------|------|------|
| 合并 Merge | ✅ | `merge_pdfs` | lopdf | ✅ 已完成 |
| 拆分 Split | ✅ | `split_pdf` | lopdf | ✅ 已完成 |
| 加密/解密 Encrypt | ✅ | `encrypt_pdf` / `decrypt_pdf` | lopdf | ✅ 已完成 |
| 压缩 Compress | ✅ | `compress_pdf` | Ghostscript | ✅ 已完成(引擎待打包) |
| 水印 Watermark | — | — | 待定 | ❌ 计划中 |
| 转换 Convert | — | — | ghostscript / 待定 | ❌ 计划中 |

## 里程碑 0 —— 引擎管道(已完成)

第一个纵向切片端到端验证了 前端 → 命令 → lopdf → 落盘文件 这条路径。

- [x] 引入 lopdf,在 `src-tauri/src/merge.rs` 实现合并算法
- [x] 接入 `tauri-plugin-dialog`,授予 `dialog:default` 权限
- [x] 在 `src-tauri/src/lib.rs` 实现 `merge_pdfs` 命令并注册
- [x] 搭建合并页面:选文件、排序、删除、选输出、执行、反馈结果
- [x] 把引擎错误以友好信息呈现到 UI(`Result<_, String>` → 状态栏)
- [x] 合并算法集成测试(两个单页 PDF → 2 页)
- [ ] 在真实 `pnpm tauri dev` 窗口里手动冒烟一次(需图形环境)

完成标准:用户从 UI 合并两个 PDF 并得到一个有效的输出文件。

## 里程碑 1 —— 补齐核心工具

每个都复用里程碑 0 的模式;唯一新增的部分是引擎调用。

- [x] **拆分 Split** —— 页码范围 / 每 N 页 → lopdf
  - [x] `parse_ranges` 解析 `1-3, 5, 8-10`,越界/倒序/非数字都给可读报错
  - [x] 逐片克隆 + `delete_pages` + 清理悬空 `Kids` + `prune_objects`
  - [x] 新增 `page_count` 命令,页面上直接显示总页数
  - [x] 7 个单元/集成测试;并用系统 PDFKit 独立验证真实 PDF 切片可打开、
        文字落在正确的页
  - [ ] 「按大小」拆分 —— 需要先能预估输出体积,推迟到压缩工具落地后再做
- [x] **加密/解密 Encrypt** —— 设置/移除密码 → lopdf
  - [x] AES-128(V4/R4);**不要**"升级"到 AES-256(V5/R6),见下方已知取舍
  - [x] 缺 `/ID` 的文档自动补一个,否则 V4 密钥推导直接失败
  - [x] 解密走 `load_with_password`,并清掉 trailer 里的 `Encrypt`
  - [x] 新增 `is_encrypted` 命令,页面据此自动切换加锁/解锁
  - [x] 7 个测试;并用系统 PDFKit 验证加密件确实需要密码、正确密码取出的
        文字与原文逐字一致、错误密码被拒
- [x] **压缩 Compress** —— 质量预设(screen/ebook/printer)→ Ghostscript
  - [x] `gs_args` 纯函数构建参数,不需要 gs 在场就能测(CI 上也跑)
  - [x] 始终传 `-dSAFER`,并有专门的测试盯住它
  - [x] 读退出码 + stderr;失败时删掉可能残留的半成品文件
  - [x] `Report` 带 `grew` 标志,输出更大时如实告知而不是假装压缩成功
  - [x] `has_compression_engine` 命令,页面在引擎缺失时直接说明
  - [ ] **把 gs 真正打包成 sidecar** —— 见下,这仍是 M3 的活
- [ ] **Ghostscript 打包(M3 前置)**
  - [ ] brew 装的 gs 链接了 12 个 homebrew dylib,不能直接分发;需要静态构建
        或用 `install_name_tool` 重写路径并把 dylib 一起塞进 app bundle
  - [ ] 在 `tauri.conf.json` 声明 `bundle.externalBin`,二进制按
        `gs-<target-triple>` 命名
  - [ ] 履行 AGPL 义务(见已知取舍)

## 里程碑 2 —— 重渲染类工具

- [ ] **水印 Watermark** —— 在每页叠加文字/图片(选定并验证渲染器)
- [ ] **转换 Convert** —— PDF→图片 与 图片→PDF

## 里程碑 3 —— 发布加固

- [ ] 把 `csp: null` 替换为真实的 Content-Security-Policy
- [ ] 为长任务(压缩、转换)加进度事件
- [ ] 为每个工具的正常路径加 E2E 冒烟测试(Playwright)
- [ ] 对 macOS 构建做代码签名 + 公证
- [ ] 跨平台引擎打包(Windows/Linux)

## 已知取舍

- **Ghostscript 是 AGPL v3,而本项目是 MIT。** 目前**尚未捆绑** —— 找不到内置
  二进制时回退到用户 `PATH` 上的 `gs`,所以现在还没有分发任何 AGPL 代码,义务
  未触发。一旦 M3 把 gs 打进 app bundle,就必须:在发布物里标注 Ghostscript
  及其 AGPL v3 许可、提供对应源码的获取方式、保留其版权声明。项目自身代码可
  继续保持 MIT(gs 是独立进程,不是链接进来的)。这条会挡住闭源分发和上架
  Mac App Store —— 真要走那条路就得买 Artifex 的商业许可。
- **压缩是有损的,而且可能压不小。** `-dPDFSETTINGS=/printer` 对图像本就低于
  300 dpi 的文档会重新编码到略大(实测 +0.2%)。工具如实报告,不假装成功。
  纯文本文档也会被重排,实测文字提取长度有 0.5% 左右的浮动(空白处理差异),
  页数和内容不变。

- **加密用 AES-128 而不是 AES-256,这是有意的。** lopdf 的 V5/R6(AES-256)
  产物无法互操作:macOS PDFKit 能验证密码,却读出 0 个字符 —— 它推导出的文件
  密钥和 lopdf 加密时用的不是同一个。lopdf 自己的 `encrypt_v5` 测试只在
  lopdf 内部往返,所以从没暴露过。V4/R4(AES-128)对 PDFKit 验证完全干净。
  别人解不开的强加密,不如能用的弱加密。要动这里,先跑外部 reader 验证。

- **拆分后体积之和大于原文件。** 嵌入字体会在每个切片里各留一份 —— 保留页引用
  什么就带走什么,这是正确性换来的。字体子集化留给压缩工具。
- **合并会丢书签。** 见 `merge.rs` 的注释,目前有意丢弃 `Outlines`。
- **拆分与合并都未处理加密文档。** 加密工具落地后再统一处理。

## 新增工具的约定

1. 在 `src/lib/tools.ts` 添加/确认条目(可用前保持 `available: false`)。
2. 创建 `src/routes/<tool>/+page.svelte` —— 选文件、设选项、执行、出结果。
3. 在 Rust 中加一个 `#[tauri::command]`;校验输入;派生引擎。
4. 在 `invoke_handler![]` 注册该命令。
5. 只授予该命令所需的新权限。
6. 置 `available: true` 并冒烟测试整条路径。
