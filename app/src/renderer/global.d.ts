import type { SucrApi } from '../shared/ipc';

declare global {
  interface Window {
    sucr: SucrApi;
  }
}

export {};
