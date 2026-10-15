export const homedir=()=>'/pi/home';
export const tmpdir=()=>'/pi/tmp';
export const platform=()=>globalThis.__sdkPlatform?.platform??'android';
export const arch=()=>globalThis.__sdkPlatform?.arch??'unknown';
export const release=()=> 'Android';
export const type=()=> 'Android';
export const EOL='\n';
export const constants={signals:{SIGTERM:15,SIGKILL:9,SIGINT:2}};
