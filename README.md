# Packo

Packo 是一款为 macOS 设计的压缩包管理工具，支持压缩、解压、预览、编辑压缩包、拖拽导出、最近文件和常用压缩格式关联。它希望把压缩包操作做得像管理普通文件夹一样自然。
> ![认可linux.do]([https://ld.xh.do/ld-badge.svg](https://linux.do/))

> 当前项目仍在快速迭代中，功能和界面可能会持续调整。
<img width="812" height="592" alt="Screenshot 2026-06-16 at 10 08 28 PM" src="https://github.com/user-attachments/assets/a06df692-e473-4d8f-b06a-e1fe824452d0" />


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
<img width="1032" height="733" alt="0fd4a8cda8df8a70e17266e4ed8e2da8" src="https://github.com/user-attachments/assets/228cfb81-cdb9-4504-a4ee-5314287824fb" />


### 压缩

当前支持：

`ZIP`、`7Z`、`TAR`、`TGZ`、`TBZ`、`TXZ`、`GZ/GZIP`、`BZ2/BZIP2`、`XZ`、`Z`、`ZSTD`、`LZMA/LZMA2`、`LZ4`
<img width="1241" height="896" alt="14e3a26ab772fe239d8602c431ce3ecd" src="https://github.com/user-attachments/assets/3560327c-cbaf-462f-b7cb-225b2a2f9de7" />

RAR 目前仅支持解压，不支持创建 RAR 压缩包。

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

## 当前限制

- 暂不支持创建 RAR 压缩包。
- 部分格式的支持能力取决于系统环境和底层命令可用性。
- Quick Look 插件和更完整的任务中心仍在计划中。
