import Axios from 'axios';
import axiosRetry from 'axios-retry';
import jsonpAdapter from 'axios-jsonp';

// 参考： https://juejin.cn/post/7053471988752318472

const CancelToken = Axios.CancelToken;

const client = Axios.create({
  // 全局基础配置
});

// 当请求失败后，自动重新请求，只有3次失败后才真正失败
axiosRetry(client, { retries: 3 });

export async function request(url, config) {
  const response = await client.request({ url, ...config });
  const result = response.data;

  // 你的业务判断逻辑
  return result;
};

export const jsonp = (url, config) => {
  return request(url, { ...config, adapter: jsonpAdapter })
}

export const withCancelToken = (fetcher) => {
  let abort

  function send(data, config) {
    cancel(); // 发送前先主动取消

    const cancelToken = new CancelToken(cancel => (abort = cancel));

    return fetcher(data, { ...config, cancelToken });
  }

  function cancel(message = 'abort') {
    if (abort) {
      abort(message)
      abort = null
    }
  }

  return [send, cancel];
};

export const generateFlatMethods = (baseUrl) => ({
  get: (url, data, config) => {
    config = config || {};
    const params = config.params || {};
    return client.get(`${baseUrl}${url}`, {
    ...config,
    params: {
      ...params,
      ...data,
    }
  })},
  del: (url, data, config) => {
    config = config || {};
    const params = config.params || {};

    return client.delete(`${baseUrl}${url}`, {
    ...config,
    params: {
      ...params,
      ...data,
    }
  })},
  put: (url, data, config) => client.put(`${baseUrl}${url}`, data, config),
  post: (url, data, config) => client.post(`${baseUrl}${url}`, data, config),
});

export default client;

