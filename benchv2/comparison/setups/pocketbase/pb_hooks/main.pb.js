/// <reference path="../pb_data/types.d.ts" />

routerAdd('GET', '/fibonacci', (c) => {
  function fibonacci(num) {
    switch (num) {
      case 0:
        return 0;
      case 1:
        return 1;
      default:
        return fibonacci(num - 1) + fibonacci(num - 2);
    }
  }
  const param = c.request.url.query().get("n");
  const n = param ? parseInt(param) : 40;
  const fib = fibonacci(n);

  return c.string(200, `${fib}`);
});
