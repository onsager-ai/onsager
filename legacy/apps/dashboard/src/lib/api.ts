// Split into per-resource modules under ./api/ (#261 Move 2).
// This file is the stable public re-export surface — all existing
// `import { ... } from '@/lib/api'` imports continue to work unchanged.
export * from './api/index';
