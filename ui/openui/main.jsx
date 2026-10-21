import React, {useEffect,useState} from 'react';
import {createRoot} from 'react-dom/client';
import {Card} from '@openuidev/react-ui/Card';
import {Button} from '@openuidev/react-ui/Button';
import {ThemeProvider} from '@openuidev/react-ui/ThemeProvider';
import '@openuidev/react-ui/styles/index.css';
import '../shared/modern.css';

function DesignPreview() {
  const [side,setSide] = useState('A');
  const [dark,setDark] = useState(true);
  const [zh,setZh] = useState(true);
  useEffect(() => {
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
    document.documentElement.lang = zh ? 'zh-CN' : 'en';
  }, [dark,zh]);
  const t = (cn,en) => zh ? cn : en;
  return <ThemeProvider mode={dark?'dark':'light'}><div className="om-modern" data-theme={dark?'dark':'light'} style={{minHeight:'100vh',padding:'24px',background:'var(--om-bg)',color:'var(--om-ink)'}}>
    <div style={{maxWidth:720,margin:'auto'}}>
      <div className="om-topbar"><div className="om-title">Ommega {side}</div><span className="om-caption">OpenUI · {t('设计预览 / 示例数据','Design preview / sample data')}</span></div>
      <div style={{display:'flex',gap:12,flexWrap:'wrap',marginBottom:20}}><Button onClick={()=>setSide(side==='A'?'B':'A')}>A / B</Button><Button onClick={()=>setDark(!dark)}>{t('切换外观','Appearance')}</Button><Button onClick={()=>setZh(!zh)}>中文 / English</Button></div>
      <Card><div className="om-hero" style={{margin:0,border:0}}><span className="om-status" data-tone="ok">{t('示例状态','Sample state')}</span><h1>{side==='A'?t('证明链路，一目了然。','Your attestation. In view.'):t('硬件证明，随时就绪。','Hardware trust. Ready to relay.')}</h1><p>{t('这份原型用于检查布局和组件层级。','Use this prototype to review layout and component hierarchy.')}</p><div className="om-flow"><div>A · KeyMint / SOTER</div><div>→ Server →</div><div>B · TEE</div></div></div></Card>
      <div className="om-grid">{['KeyMint','SOTER',t('模块版本','Module version')].map((label,i)=><Card key={label}><div className="om-card"><div className="om-eyebrow">{label}</div><div className="om-stat">{['PID 1924',t('软件 TA','Software TA'),'1.8.0'][i]}</div><p className="om-description">{t('示例数据','Sample data')}</p></div></Card>)}</div>
      <p className="om-caption">{t('固定组件、本地渲染；不接收真实配置或凭据。','Fixed components, locally rendered. No live configuration or credentials.')}</p>
    </div>
  </div></ThemeProvider>;
}
createRoot(document.getElementById('root')).render(<DesignPreview/>);
