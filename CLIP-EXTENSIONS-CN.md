桌面端应用全局划词插件
----------------------

<p align="center">
    <br> <a href="CLIP-EXTENSIONS.md">English</a> | 中文
</p>

划词翻译是本软件的杀手锏功能，对于浏览器插件来说，浏览器提供了简单的 API 来获得选中的文本，但是对于桌面端应用来说，各个操作系统都没有统一的 API 来获得选中的文本。

通常都是使用剪切板来获得选中的文本，但是这样会在一些应用中造成剪切板混乱等 bug，在 macOS 中还会因为没有选中文本就按下 cmd+c 快捷键导致发出警告声。幸运的是，各个操作系统中已经有很多成熟的划词软件，而且它们有完善的插件机制，所以 NextAI Translator 特地开发了针对这些划词软件的插件来让用户无痛地使用划词翻译。

# macOS

## PopClip

[PopClip](https://pilotmoon.com/popclip/) 是 macOS 上成熟的划词软件，它提供了完善的插件机制，我们提供了它的插件，安装步骤如下：

* 1. 下载并安装 [PopClip](https://pilotmoon.com/popclip/)
* 2. 下载 [nextai-translator.popclipextz](https://github.com/nextai-translator/nextai-translator/releases/latest/download/nextai-translator.popclipextz)
* 3. 双击下载完毕的 nextai-translator.popclipextz，点击弹出窗口中的 Install "NextAI Translator" 按钮即可安装完毕
    
    <p align="center">
        <img width="400" src="https://user-images.githubusercontent.com/1206493/240260692-8af6141a-3dba-4775-921d-505223addf9e.png" />
    </p>

* 4. 在 PopClip 中开启 NextAI Translator
    
    <p align="center">
        <img width="400" src="https://user-images.githubusercontent.com/1206493/240258859-c4f2ec91-255f-414c-a4a4-aca25fceb0b5.png" />
    </p>

* 5. 效果如下

    <p align="center">
        <img width="600" src="https://user-images.githubusercontent.com/1206493/240355949-8f41d98d-f097-4ce4-a533-af60e1757ca1.gif" />
    </p>

# Windows

## SnipDo

* 1. 下载并安装 [SnipDo](https://apps.microsoft.com/store/detail/snipdo/9NPZ2TVKJVT7)
* 2. 下载 [nextai-translator.pbar](https://github.com/nextai-translator/nextai-translator/releases/latest/download/nextai-translator.pbar)
* 3. 双击下载完毕的 nextai-translator.pbar 即可安装
* 4. 在 SnipDo 的设置页面中启用 NextAI Translator

    <p align="center">
        <img width="200" src="https://github.com/nextai-translator/nextai-translator/assets/1206493/09d66943-06db-4ba7-b217-a434c33cc8aa" />
    </p>

    建议只保留 NextAI Translator:
  
    <p align="center">    
        <img width="600" src="https://github.com/nextai-translator/nextai-translator/assets/1206493/76b619d9-e63d-4d67-a32c-a0d2d6923558" />
    </p>


* 5. 效果如下

    <p align="center">
        <img width="600" src="https://user-images.githubusercontent.com/1206493/240358161-2788eb97-d00b-4808-aa86-a7fcfe3f71dd.gif" />
    </p>

# Linux

Linux 上目前没有 PopClip / SnipDo 那样的通用划词软件，但它们的插件本质上只是把选中文本通过 IPC socket 交给桌面端：

```sh
curl -d "$TEXT" --unix-socket /tmp/openai-translator.sock http://nextai-translator
```

这个 socket 在 Linux 版上同样存在，所以 `clip-extensions/linux/nextai-translator.sh` 补上了唯一缺的部分——读取选区：

* 1. 安装：`install -Dm755 clip-extensions/linux/nextai-translator.sh ~/.local/bin/nextai-translate-selection`
* 2. 在合成器里绑一个键（Hyprland 示例，写进 `~/.config/hypr/bindings.lua`）：

```lua
o.bind("ALT + T", "Translate selection", "nextai-translate-selection")
```

* 3. 选中文本 → 按快捷键 → 翻译窗口弹出。app 没在运行时脚本会自己拉起它

脚本优先读 Wayland 的 PRIMARY selection（`wl-paste --primary`），失败才回退 X11 的 `xclip`。

如果不想依赖 curl / 脚本，桌面端也内置了同样的能力：

```sh
nextai-translator --translate-selection
```

它直接取 PRIMARY selection 并打开翻译窗口，同样适合直接绑到快捷键上。
