import { invoke } from '@tauri-apps/api/core'
import icon from '@/common/assets/images/icon.png'
import { useTheme } from '@/common/hooks/useTheme'
import { BaseProvider } from 'baseui-sd'

export function ThumbWindow() {
    const { theme } = useTheme()

    return (
        <BaseProvider theme={theme}>
            <div
                className='thumb'
                style={{
                    background: theme.colors.backgroundPrimary,
                    cursor: 'pointer',
                }}
                // Upstream detects "click on the icon" inside the macOS/Windows
                // mouse hook, which does not exist on Linux. The icon window is a
                // webview, so handle the click here instead.
                onMouseDown={() => {
                    void invoke('thumb_clicked')
                }}
            >
                <img
                    draggable={false}
                    style={{
                        display: 'block',
                        width: '100%',
                        height: '100%',
                    }}
                    src={icon}
                />
            </div>
        </BaseProvider>
    )
}
