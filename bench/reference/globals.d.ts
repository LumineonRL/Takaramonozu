// This is used for logging and it's slow as heck when enabled and would not result in a fair comparison
// so this constant, set by the build script, forces it off.
declare const VITE_APP_DIAGNOSTICS: boolean

declare module 'vue' {
    export function reactive<T>(value: T): T
    export function toRaw<T>(value: T): T
    export function ref<T>(value: T): { value: T }
}
