/** mammoth 浏览器包无自带类型，这里声明本项目用到的最小 API 面 */
declare module "mammoth/mammoth.browser.js" {
  interface MammothImage {
    contentType: string;
    read(mode: "base64"): Promise<string>;
  }

  interface MammothMessage {
    message: string;
  }

  interface MammothConvertResult {
    value: string;
    messages: MammothMessage[];
  }

  const mammoth: {
    convertToHtml(
      input: { arrayBuffer: ArrayBuffer },
      options?: {
        convertImage?: unknown;
        styleMap?: string[];
      },
    ): Promise<MammothConvertResult>;
    images: {
      imgElement(
        cb: (image: MammothImage) => Promise<{ src: string }>,
      ): unknown;
    };
  };

  export default mammoth;
}
