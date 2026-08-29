// document表示整个HTML文档对象，addEventListener为文档添加事件监听器
// 'DOMContentLoaded'事件在HTML文档完全加载并解析完成后触发，不等待图片等外部资源加载
// 使用此事件确保DOM元素已准备好，可以安全地操作DOM
document.addEventListener('DOMContentLoaded', function() {
    // console.log向浏览器开发者工具的控制台输出日志信息，用于调试和验证代码执行
    // 此处输出提示信息，确认页面DOM已成功加载
    console.log('Page loaded successfully');
// 匿名函数结束，作为事件回调函数
});