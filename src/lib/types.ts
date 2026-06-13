// T-000102: split by domain into types/<name>.ts. This file is a pure barrel —
// re-exports all sub-module symbols so existing `from '$lib/types'` imports
// (40 callers across components and stores) compile unchanged.

export * from './types/core';
export * from './types/bugs';
export * from './types/bundle';
export * from './types/dashboard';
export * from './types/deploy';
export * from './types/graph';
export * from './types/stats';
export * from './types/sync';
export * from './types/tasks';
export * from './types/templates';
export * from './types/timeline';
