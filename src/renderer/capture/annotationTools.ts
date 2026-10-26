import { ArrowUpRight, Circle, Grid3X3, MousePointer2, Pencil, Square, Type } from 'lucide-react';

export const ANNOTATION_TOOLS = [
  ['select', MousePointer2, '选择与调整'], ['rectangle', Square, '矩形'], ['ellipse', Circle, '椭圆'],
  ['arrow', ArrowUpRight, '箭头'], ['pen', Pencil, '画笔'], ['text', Type, '文字'], ['mosaic', Grid3X3, '马赛克'],
] as const;
