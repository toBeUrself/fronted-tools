### 模块化

----

> 模块化是指将一个复杂的系统分解为多个模块以方便编码。

很久以前，开发网页要通过命名空间的方式来组织代码，例如 jQuery 库将它的 API 都放在了  window.$ 下，在加载完 jQuery 后，其他模块再通过 window.$ 去使用 jQuery。这样做有很多问题，其中包括：

* 命名空间冲突，两个库可能会使用同一个名称，例如 Zepto(zepto.com) 也是放在 window.$ 下；
* 无法合理地管理项目的依赖和版本；
* 无法方便地控制依赖的加载顺序；

当项目变大，这种方式将变得难以维护，需要用模块化的思想来组织代码。

### 一 、模块化优点：

1. 提升开发效率：代码方便重用，别人开发的模块直接拿过来就可以使用，不需要重复开发法类似的功能。

2. 方便后期维护：代码方便重用，别人开发的模块直接拿过来就可以使用，不需要重复开发法类似的功能。

所以总结来说，在生产角度，模块化开发是一种生产方式，这种方式生产效率高，维护成本低。从软件开发角度来说，模块化开发是一种开发模式，写代码的一种方式，开发效率高，方便后期维护。

### 二、模块化规范

1. 服务器端规范主要是CommonJS，node.js用的就是CommonJS规范。
2. 客户端规范主要有：AMD（异步模块定义，推崇依赖前置）、CMD（通用模块定义，推崇依赖就近）。AMD规范的实现主要有RequireJS，CMD规范的主要实现有SeaJS。但是SeaJS已经停止维护了，因为在ES6中已经有了模块化的实现，随着ES6的普及，第三方的模块化实现将会慢慢的淘汰。
3. 浏览器能够最优化加载模块，使它比使用库更有效率：使用库通常需要做额外的客户端处理。

### 三、CommonJS

[参考文章](https://juejin.cn/post/6844903665547870216)

```js
function Module(id, parent){
    this.id = id;
    this.exports = {};
    this.parent = parent;
    this.filename = null;
    this.loaded = false;
    this.children = []
}

module.exports = Module;
var module = new Module(filename, parent)
```

## 所有的模块都是Module的实例 ##

* module.id 模块的识别符，通常是带有绝对路径的模块文件名。
* module.filename 模块的文件名，带有绝对路径。
* module.loaded 返回一个布尔值，表示模块是否已经完成加载。
* module.parent 返回一个对象，表示调用该模块的模块。
* module.children 返回一个数组，表示该模块要用到的其他模块。
* module.exports 表示模块对外输出的值

1.3 模块实例的 require 方法

```js
Module.prototype.require = function(path){
  return Module._load(path, this)  
}
```

由此可知，require 并不是全局命令，而是每个模块提供的一个内部方法，也就是说，只有在模块内部才能使用require命令，（唯一的例外是REPL 环境）。另外，require 其实内部调用 Module._load 方法。

下面来看 Module._load 的源码。

```js
Module._load = function(request, parent, isMain) {

  //  计算绝对路径
  var filename = Module._resolveFilename(request, parent);

  //  第一步：如果有缓存，取出缓存
  var cachedModule = Module._cache[filename];
  if (cachedModule) {
    return cachedModule.exports;
  }
  
  // 第二步：是否为内置模块
  if (NativeModule.exists(filename)) {
    return NativeModule.require(filename);
  }

  // 第三步：生成模块实例，存入缓存
  var module = new Module(filename, parent);
  Module._cache[filename] = module;

  // 第四步：加载模块
  try {
    module.load(filename);
    hadException = false;
  } finally {
    if (hadException) {
      delete Module._cache[filename];
    }
  }

  // 第五步：输出模块的exports属性
  return module.exports;
};
```

下面就是 module.load 方法的源码。

```js
Module.prototype.load = function (filename) {
    var extension = path.extname(filename)  || 'js'
    if(!Module._extensions[extensions]) extension = '.js'
    Module._extensions[extension](this, filename)
    this.loaded = true
}
```

上面代码中，首先确定模块的后缀名，不同的后缀名对应不同的加载方法。下面是.js和.json后缀名对应的处理方法。

```js
Module._extensions['.js'] = function(module, filename) {
  var content = fs.readFileSync(filename, 'utf8');
  module._compile(stripBOM(content), filename);
};

Module._extensions['.json'] = function(module, filename) {
  var content = fs.readFileSync(filename, 'utf8');
  try {
    module.exports = JSON.parse(stripBOM(content));
  } catch (err) {
    err.message = filename + ': ' + err.message;
    throw err;
  }
};
```

这里只讨论 js 文件的加载。首先，将模块文件读取成字符串，然后剥离 utf8 编码特有的BOM文件头，最后编译该模块。

```js
Module.prototype._compile = function(content, filename) {
  var self = this;
  var args = [self.exports, require, self, filename, dirname];
  return compiledWrapper.apply(self.exports, args);
};
```

也就是说，模块的加载实质上就是，注入exports、require、module三个全局变量，然后执行模块的源码，然后将模块的 exports 变量的值输出。


### requireJs原理分析

> 在require中，根据AMD(Asynchronous Module Definition)的思想，即异步模块加载机制，其思想就是将代码分为一个一个模块分块加载，来提高代码的重用。

简单流程概括
1. 我们在使用requireJs时，都是在页面上只引入一个require.js，把data-main指向我们的main.js
2. 运行main.js时，执行里面的require和define方法，requireJs会把这些依赖和回调方法都用一个数据结构存起来
3. 当页面加载时，requireJs 会根据依赖预先先把需要的js通过document.createElement的方法引入dom中，这样，被引入dom的script便会执行
4. 依赖的js也是根据requireJs的规范来写的，所以他们也会有define或者require方法，同样类似第二步这样循环向上查找依赖，同样会存起来
5. 当js需要用到依赖返回的结果,requireJs会把保存的方法拿出来并且运行