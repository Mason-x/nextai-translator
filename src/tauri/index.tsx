import './legacy-tauri-ipc'
import { createRoot } from 'react-dom/client'
import { App } from './App'
import { installWheelScrollFallback } from './wheelScrollFallback'

import '../common/i18n.js'

installWheelScrollFallback()

const root = createRoot(document.getElementById('root')!)

root.render(<App />)
