export type ConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'error';

export interface ThemeInfo {
  name: string;
  background: [number, number, number];
  foreground: [number, number, number];
  normal: [[number, number, number], [number, number, number], [number, number, number], [number, number, number], [number, number, number], [number, number, number], [number, number, number], [number, number, number]];
  bright: [[number, number, number], [number, number, number], [number, number, number], [number, number, number], [number, number, number], [number, number, number], [number, number, number], [number, number, number]];
}
