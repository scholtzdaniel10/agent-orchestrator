import './assets/main.css'
import.meta.glob('./assets/xp/*.css', { eager: true })

import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import TerminalWindow from './TerminalWindow'
import { applyTheme, loadTheme } from './theme'

applyTheme(loadTheme())

const terminalId = new URLSearchParams(window.location.search).get('terminal')

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {terminalId !== null && terminalId !== '' ? <TerminalWindow id={terminalId} /> : <App />}
  </StrictMode>
)
