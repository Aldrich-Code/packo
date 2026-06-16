# Packo

Packo 是一款为 macOS 设计的压缩包管理工具，支持压缩、解压、预览、编辑压缩包、拖拽导出、最近文件和常用压缩格式关联。它希望把压缩包操作做得像管理普通文件夹一样自然。

> 当前项目仍在快速迭代中，功能和界面可能会持续调整。

## 功能特性

- 压缩和解压：支持常见压缩格式的创建、解压和批量处理。
- 压缩包预览：打开压缩包后以文件树查看内部目录、文件大小、类型和修改时间。
- 压缩包编辑：支持在压缩包内重命名、删除、新建文件夹、追加文件或文件夹。
- 拖拽工作流：支持把压缩包内的文件或文件夹直接拖出到 Finder，也支持拖入文件添加到压缩包。
- 密码处理：支持加密压缩包的解压密码输入，以及支持格式的压缩密码设置。
- 分卷压缩包：支持识别和处理 `.001`、`.z01`、`.part1.rar` 等常见分卷入口。
- macOS 集成：支持文件关联、Dock 拖拽、Finder 服务菜单、中文菜单栏和系统图标。
- 最近文件：记录打开、预览、拖入和压缩过的压缩包，方便快速回到最近任务。
- 安全检查：解压前检测不安全路径、隐藏文件和可执行文件，降低误操作风险。
- macOS 元数据过滤：可选择排除 `.DS_Store`、`__MACOSX`、资源分叉和扩展属性等内容。

## 支持格式

### 解压

Packo 目标支持以下格式中的常见场景：

`ZIP`、`RAR`、`7Z`、`GZ/GZIP`、`BZ2/BZIP2`、`XZ`、`TAR`、`TGZ`、`TBZ`、`TXZ`、`LZH/LHA`、`Z`、`ZSTD`、`LZMA/LZMA2`、`LZ4`、`ISO`、Office 文档容器，以及常见分卷压缩包。

### 压缩

当前支持：

`ZIP`、`7Z`、`TAR`、`TGZ`、`TBZ`、`TXZ`、`GZ/GZIP`、`BZ2/BZIP2`、`XZ`、`Z`、`ZSTD`、`LZMA/LZMA2`、`LZ4`

RAR 目前仅支持解压，不支持创建 RAR 压缩包。

## 截图

可以把截图放到 `static/screenshots/`，然后在这里引用：

```md
![Packo 主界面](static/screenshots/home.png)
![压缩包文件树](static/screenshots/archive-tree.png)
```

## 技术栈

- Tauri 2
- Svelte 5
- SvelteKit
- TypeScript
- Rust

## 开发环境

需要先安装：

- macOS
- Node.js
- Rust
- Tauri 依赖环境

安装依赖：

```bash
npm install
```

启动开发环境：

```bash
npm run tauri dev
```

前端检查：

```bash
npm run check
```

构建 macOS App：

```bash
npm run tauri build -- --bundles app
```

构建完成后，应用会生成在：

```text
src-tauri/target/release/bundle/macos/Packo.app
```

## GitHub 上传建议

创建仓库时建议：

- Repository name：`packo`
- Description：`Packo 是一款 macOS 压缩管理工具，支持压缩、解压、预览、编辑压缩包、拖拽导出、最近文件和常用格式关联。`
- Add README：如果本地已经有这个 README，可以不在 GitHub 创建页重复勾选，避免冲突。
- Add .gitignore：本地已经有 `.gitignore`，可以不重复添加。
- License：如果准备开源，可以选择 MIT；如果暂时不想授权别人使用，可以先不加 License。

## 当前限制

- 暂不支持创建 RAR 压缩包。
- 部分格式的支持能力取决于系统环境和底层命令可用性。
- Quick Look 插件和更完整的任务中心仍在计划中。

## 路线图

详细功能路线图见 [PACKO_ROADMAP.md](./PACKO_ROADMAP.md)。
