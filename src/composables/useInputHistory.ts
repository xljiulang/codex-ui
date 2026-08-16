/** 用户输入历史（仅内存），供向上/向下键选择，行为类似 Linux shell */
export function useInputHistory(limit = 100) {
  const sentHistory: string[] = [];
  let historyIndex = -1;

  /** 发送后记录输入并回到新输入态 */
  function push(text: string) {
    sentHistory.push(text);
    if (sentHistory.length > limit) sentHistory.shift();
    historyIndex = -1;
  }

  /** ↑：返回上一条历史文本；无历史返回 null（调用方仍应消费按键） */
  function prev(): string | null {
    if (!sentHistory.length) return null;
    if (historyIndex === -1) historyIndex = sentHistory.length - 1;
    else if (historyIndex > 0) historyIndex--;
    return sentHistory[historyIndex];
  }

  /** ↓：返回下一条历史文本；已到末尾返回 ""（清空输入）；未开始浏览返回 null */
  function next(): string | null {
    if (historyIndex === -1) return null;
    historyIndex++;
    if (historyIndex >= sentHistory.length) {
      historyIndex = -1;
      return "";
    }
    return sentHistory[historyIndex];
  }

  return { push, prev, next };
}
