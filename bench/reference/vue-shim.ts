// Minimal stand-ins for the Vue run-time.
// reactive is normally used in database/index.ts for rendering but we don't need to render.
// ref is normally a flag used for rendering
// toRaw normally unwraps the reactive objects.

export function reactive<T>(value: T): T {
    return value
}

export function toRaw<T>(value: T): T {
    return value
}

export function ref<T>(value: T) {
    return { value }
}
