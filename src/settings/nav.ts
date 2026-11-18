// 页面跳转（各页面里的"管理 →"之类的链接用），具体实现由 main.ts 注册

let impl: (tab: string) => void = () => {};

export function setNav(fn: (tab: string) => void) {
  impl = fn;
}

export function go(tab: string) {
  impl(tab);
}
