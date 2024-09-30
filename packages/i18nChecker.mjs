import fs from 'fs';
import path from 'path';
import * as parser from '@babel/parser';
// import traverse from '@babel/traverse';
import _traverse from '@babel/traverse';
const traverse = _traverse.default;

const isPro = process.env.NEXT_PUBLIC_ENV === 'pro';

// 语种全部key
let langSet = null;
let currentPage = '';
let currentFile = '';
let fileStack = [];
let fileStackSet = new Set();
// 入口文件及其依赖
const pageMap = new Map(); // page作为key,  [Set, Set] 作为值，保存依赖和lang keys
const cacheFileMap = new Map(); // file作为key,  [Map, Set] 作为值，保存依赖和lang keys
let fileDepSet = null;
let pageLangSet = null;
// 每个文件使用的key
const fileKeysMap = new Map();
// 依赖目录
let nodeModulesNameSet = null;
const jsFileTail = ['.js', '.jsx', '.ts', '.tsx'];

const libDirPath = './node_modules';
const baseComponents = ['@xt/Foot', '@xt/Nav'];

const isBase = (dirname) => dirname === '@xt';
const isJsFile = (fileName) => {
  const extname = path.extname(fileName);

  return jsFileTail.includes(extname);
};

export const getPageRoad = (page) => {
  const pageTail = page.split('server')[1];

  const realPage = path.resolve('./src', `.${pageTail}`);
  if (fs.existsSync(realPage)) {
    return realPage;
  }

  let readlPage = '';
  const pageName = pageTail.split('.')[0];
  ['.jsx', '.js', '.tsx', '.ts'].forEach(tail => {
    let pageRoad = path.resolve(`./src${pageName}/index${tail}`);

    if (fs.existsSync(pageRoad)) {
      readlPage =  pageRoad;
    }
  });

  return readlPage;
};

const getFileFromImport = (road) => {
  let filePath = road;
  if (fs.existsSync(road)) {
    const stat = fs.lstatSync(road);

    if (stat.isDirectory()) {
      jsFileTail.forEach(tail => {
        const temp = road + '/index' + tail;
  
        if (fs.existsSync(temp)) {
          filePath = temp;
        }
      });
    }
  } else {
    jsFileTail.forEach(tail => {
      const temp = road + tail;
      const temp1 = road + '/index' + tail;

      if (fs.existsSync(temp)) {
        filePath = temp;
      } else if (fs.existsSync(temp1)) {
        filePath = temp1;
      }
    });
  }

  return filePath;
};

// 在dir中递归找js文件并交给handler
const dirWalker = (road, handler) => {
  const absolutePath = path.resolve(road);
  const stat = fs.lstatSync(absolutePath);
  if (stat.isFile() && isJsFile(absolutePath)) {
    return handler(absolutePath);
  }

  // console.log('[Walker]', absolutePath, road);
  const files = fs.readdirSync(absolutePath);

  files.forEach(file => {
    const fileName = `${absolutePath}/${file}`;
    const stat = fs.lstatSync(fileName);

    // console.log('[Walker]', fileName);
  
    if (stat.isDirectory()) {
      // 层级不会太深，直接递归
      dirWalker(fileName, handler);
    } else {
      if (isJsFile(fileName)) {
        handler(fileName);
      }
    }
  });
};

// js文件处理
const fileWalker = (road) => {
  try {
    const fileContent = fs.readFileSync(road, {
      encoding: 'utf-8',
    });
    
    const ast = parser.parse(fileContent, {
      sourceType: 'module',
      plugins: ['jsx', 'typescript', 'decorators-legacy'],
      allowUndeclaredExports: true,
    });

    traverse(ast, {
      ImportDeclaration(code) {
        const { node } = code;
        const value = node.source.value;
        const dirName = value?.split('/')?.[0];

        if (isBase(dirName) || !nodeModulesNameSet.has(dirName)) {
          let impRoad = '';
          if (dirName === '.' || dirName === '..') {
            impRoad = path.resolve(road, '../', value);
          } else if (dirName === '@') {
            impRoad = path.resolve('./src', value.replace('@/', ''));
          } else if (isBase(dirName)) { // @xt组件处理
            impRoad = path.resolve(libDirPath, value, './src');
          } else {
            impRoad = path.resolve('./src', value);
          }

          const filePath = getFileFromImport(impRoad);
          // console.log('ImportDeclaration: ', filePath);
          if (isJsFile(filePath) && !fileStackSet.has(filePath)) {
            fileStackSet.add(filePath);
            fileStack.push(filePath);
          }
        }
      },
      StringLiteral(code) {
        // Nav & footer不全
        if (langSet.has(code.node.value)) {
          if (!fileKeysMap.has(road)) {
            fileKeysMap.set(road, new Set());
          }
  
          const roadSet = fileKeysMap.get(road);
          // console.log(road, ': ', code.node.value);
  
          roadSet.add(code.node.value);
          pageLangSet.add(code.node.value); // 当前page依赖下的多语言都放入pageLangSet
        }
      }
    });
  } catch(err) {
    console.error('traverse ast报错了：', err);
  }
};

const checker = (
  langJson,
  manuals = [], // 手动注入的i18n key
  sourcePaths = ['./src/pages'], // 默认检测src下面所有js
) => {
  console.time('【i18nChecker】');
  if (pageMap.size) {
    console.timeEnd('【i18nChecker】');
    return pageMap;
  }

  let baseComponentLangs = [];
  try {
    baseComponentLangs = baseComponents.reduce((pre, next) => {
      const langFile = path.resolve('./node_modules', next, './src/lang/cn.json');
      console.log('@xt langFile', langFile);
      const tempLangs = JSON.parse(fs.readFileSync(langFile));

      const res = [...pre, ...Object.keys(tempLangs)];

      return res;
    }, []);
  } catch {
    console.error('读取@xt组件多语言失败');
  }
  // console.log('baseComponentLangs', baseComponentLangs);

  try {
    if (langJson) {
      langSet = new Set([...baseComponentLangs, ...Object.keys(langJson)]);
    } else {
      const tempLangs = JSON.parse(fs.readFileSync(path.resolve('./src/locales/lang/cn.json')));
      langSet = new Set([...baseComponentLangs, ...Object.keys(tempLangs)]);
    }
  } catch {
    console.error('必须传参语种JSON');
  }

  // 保存node_modules目录
  const absolutePath = path.resolve(libDirPath);
  const names = fs.readdirSync(absolutePath);
  nodeModulesNameSet = new Set(names);

  sourcePaths.forEach(dir => {
    dirWalker(dir, (page) => {
      if (!pageMap.has(page)) {
        pageMap.set(page, [new Set(), new Set(manuals)]);
      }
    });
  });

  // page处理
  Array.from(pageMap.keys()).forEach(page => {
    currentPage = page;
    const pageValue = pageMap.get(currentPage);
    fileDepSet = pageValue[0];
    pageLangSet = pageValue[1];

    fileStack = [page];
    fileStackSet = new Set(fileStack);

    while(fileStack.length) {
      currentFile = fileStack.pop();
      if (currentFile !== currentPage) {
        fileDepSet.add(currentFile);
      }

      // if (!cacheFileMap.has(currentFile)) {
      //   cacheFileMap.set(currentFile, new Set())
      // } else {

      // }

      // console.log('pageQueue', currentFile, fileStack.length);
      fileWalker(currentFile);
    }
  });

  if (isPro) {
    const fileName = path.resolve('./src/utils/langKeys.json');
    const entriys = Object.fromEntries(pageMap);

    const pageObj = {};
    Object.keys(entriys).forEach((page) => {
      const temp = entriys[page];

      pageObj[page] = [
        Array.from(temp[1]),
        Array.from(temp[0]),
      ];
    });

    const fileContent = JSON.stringify(pageObj, null, 2);

    fs.writeFileSync(fileName, fileContent, 'utf-8');
  }

  console.timeEnd('【i18nChecker】');
  return pageMap;
};

if (isPro) {
  checker();
}

export default checker;
