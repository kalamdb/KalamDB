/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_CHAT_ROOM: string
  readonly VITE_KALAM_URL: string
  readonly VITE_KALAM_USER: string
  readonly VITE_KALAM_PASSWORD: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
