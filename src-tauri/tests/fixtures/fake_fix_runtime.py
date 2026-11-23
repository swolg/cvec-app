#!/usr/bin/env python3
"""Deterministic agent transports. No model request or credential access."""
import json, os, sys, time, pathlib, urllib.request, urllib.error, socket, subprocess

def emit(value):
    print(json.dumps(value), flush=True)

def write_pid_marker(filename, pid):
    path = pathlib.Path(filename)
    pending = path.with_name(path.name + '.tmp')
    pending.write_text(str(pid))
    pending.replace(path)

def block_stage(stage):
    if os.environ.get('CVEC_FAKE_FIX_BLOCK_STAGE') == stage:
        if os.environ.get('CVEC_FAKE_FIX_WITH_DESCENDANT') == '1':
            child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)'])
            write_pid_marker(os.environ['CVEC_FAKE_FIX_MARKER']+'.helper', child.pid)
        # The Rust test waits for this path to exist before reading it.
        write_pid_marker(os.environ['CVEC_FAKE_FIX_MARKER'], os.getpid())
        while True: time.sleep(.05)

def mcp(server, method, params=None, auth=None, origin=False):
    headers = {'Content-Type':'application/json', 'Authorization':auth or server['auth']}
    if origin: headers['Origin']='https://untrusted.example'
    request=urllib.request.Request(server['url'], data=json.dumps({'jsonrpc':'2.0','id':7,'method':method,'params':params or {}}).encode(),headers=headers)
    try:
        with urllib.request.urlopen(request,timeout=5) as response: return json.load(response)
    except urllib.error.HTTPError as error: return {'httpStatus':error.code}

def fix(server, prompt):
    assert mcp(server,'tools/list',auth='Bearer wrong')['httpStatus']==403
    assert mcp(server,'tools/list',origin=True)['httpStatus']==403
    tools=mcp(server,'tools/list')['result']['tools']
    assert len(tools)==7
    assert mcp(server,'tools/call',{'name':'shell','arguments':{'command':'git push'}})['result']['isError']
    assert mcp(server,'tools/call',{'name':'cvec_workspace_write_file','arguments':{'path':'../outside.txt','content':'bad'}})['result']['isError']
    if '[[WAIT]]' in prompt.split('<untrusted_scan_evidence>')[0]:
        block_stage('active-fix')
        time.sleep(120)
    result=mcp(server,'tools/call',{'name':'cvec_workspace_write_file','arguments':{'path':'app.ts','content':'export const fixed = true;\n'}})
    assert result['result']['isError'] is False, result
    return 'Applied the focused worktree change using the selected provider.'

mode=sys.argv[1]
args=sys.argv[2:]
if mode=='grok':
    if 'inspect' in args:
        block_stage('grok-inspect')
        emit({'projectTrusted':True,'permissions':{'sources':[str(pathlib.Path.cwd()/'.grok/config.toml')+' (config)'],'loaded':13,'skipped':[]},'hooks':[],'plugins':[],'mcpServers':[]})
        sys.exit(0)
    assert '--trust' in args and '--no-leader' in args and args[args.index('--tools')+1]==''
    server=None
    for line in sys.stdin:
        request=json.loads(line);method=request.get('method');params=request.get('params',{});rid=request.get('id')
        if method=='initialize':
            result={'protocolVersion':1,'agentCapabilities':{'mcpCapabilities':{'http':True},'_meta':{'x.ai/hooks':{'blockingEvents':['pre_tool_use']}}},'authMethods':[{'id':'cached_token'}]}
        elif method=='authenticate': result={}
        elif method=='session/new':
            assert params['_meta']['x.ai/hooks']['pre_tool_use'][0]['hookCallbackIds']==['cvec-tools']
            assert '"Bash"' in pathlib.Path(params['cwd'],'.grok/config.toml').read_text()
            item=params['mcpServers'][0];server={'url':item['url'],'auth':item['headers'][0]['value']}
            result={'sessionId':'fixture-grok'}
        elif method=='session/set_model': result={}
        elif method=='session/prompt':
            if '[[LICENSE_ERROR]]' in params['prompt'][0]['text'].split('<untrusted_scan_evidence>')[0]:
                emit({'jsonrpc':'2.0','id':rid,'error':{'code':-32000,'message':'Synthetic account has no active model license.'}})
                continue
            for name, decision in [('read_file','deny'),('cvec__cvec_workspace_write_file','continue'),('other__cvec_workspace_write_file','deny')]:
                emit({'jsonrpc':'2.0','id':900,'method':'_x.ai/hooks/run','params':{'sessionId':'fixture-grok','hookCallbackId':'cvec-tools','hookEventName':'pre_tool_use','toolName':name}})
                answer=json.loads(sys.stdin.readline());assert answer['result']['decision']==decision
            emit({'jsonrpc':'2.0','id':901,'method':'session/request_permission','params':{'toolCall':{'title':'git push'}}})
            assert json.loads(sys.stdin.readline())['result']['outcome']['outcome']=='cancelled'
            text=fix(server,params['prompt'][0]['text'])
            emit({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'fixture-grok','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':text}}}})
            result={'stopReason':'end_turn'}
        else: continue
        emit({'jsonrpc':'2.0','id':rid,'result':result})
elif mode=='opencode':
    if '--version' in args:
        block_stage('opencode-version')
        print('2.0.32');sys.exit(0)
    if args[0]=='serve':
        if os.environ.get('CVEC_FAKE_FIX_MARKER'):
            write_pid_marker(os.environ['CVEC_FAKE_FIX_MARKER']+'.server', os.getpid())
        block_stage('opencode-server')
        config=json.loads(os.environ['OPENCODE_CONFIG_CONTENT'])
        pathlib.Path('fixture-config.json').write_text(json.dumps(config))
        sock=socket.socket();sock.bind(('127.0.0.1',0));sock.listen()
        emit({'url':'http://127.0.0.1:'+str(sock.getsockname()[1])})
        sys.stdin.read();sys.exit(0)
    config=json.loads(pathlib.Path('fixture-config.json').read_text())
    if args[:2]==['api','agent.list']:
        block_stage('opencode-agent')
        emit({'data':[{'id':'cvec-fix','permissions':config['agents']['cvec-fix']['permissions']}]})
    elif args[0]=='models':
        block_stage('opencode-models')
        print('fixture/model')
    elif args[:2]==['api','session.message.list']:
        block_stage('opencode-completion')
        assert 'sessionID=fixture-opencode' in args and 'order=desc' in args and 'limit=2' in args
        emit(json.loads(pathlib.Path('fixture-completion.json').read_text()))
    elif args[0]=='run':
        if os.environ.get('CVEC_FAKE_FIX_MARKER'):
            pathlib.Path(os.environ['CVEC_FAKE_FIX_MARKER']+'.run').touch()
        # Like the native CLI, `run` consumes piped input before its argv task.
        # CVEC supplies the task in argv, so stdin must already be at EOF.
        assert sys.stdin.read() == ''
        if '[[NO_OUTPUT]]' in args[-1]: sys.exit(0)
        if '[[PROVIDER_ERROR]]' in args[-1]:
            emit({'type':'error','error':{'message':'Connection refused by the configured model endpoint.\nPrivate diagnostic omitted.'}})
            sys.exit(1)
        if '[[AUTH_ERROR]]' in args[-1]:
            emit({'type':'error','error':{'message':'api_key=synthetic-sensitive-value'}})
            sys.exit(1)
        assert args[args.index('--agent')+1]=='cvec-fix'
        permissions=config['permissions'];assert permissions[0]=={'action':'*','resource':'*','effect':'deny'}
        assert all(rule['action'].startswith('cvec_cvec_workspace_') for rule in permissions[1:])
        item=config['mcp']['servers']['cvec'];server={'url':item['url'],'auth':item['headers']['Authorization']}
        text=fix(server,args[-1])
        incomplete='[[INCOMPLETE_FINISH]]' in args[-1]
        gap='[[NATIVE_FINISH_GAP]]' in args[-1] or os.environ.get('CVEC_FAKE_FIX_BLOCK_STAGE')=='opencode-completion'
        reason='length' if incomplete else 'stop'
        outcome='failed' if '[[FAILED_COMPLETION]]' in args[-1] else 'succeeded'
        pathlib.Path('fixture-completion.json').write_text(json.dumps({'data':[
            {'id':'msg_fixture-idle','type':'idle','outcome':outcome},
            {'id':'msg_fixture-assistant','type':'assistant','finish':reason},
        ]}))
        # 2.0.32 can emit only the earlier tool-call finish before reconciling
        # the final text; its persisted messages still hold the final outcome.
        session_fields={} if '[[NO_SESSION_ID]]' in args[-1] else {'sessionID':'fixture-opencode'}
        finish_session={'sessionID':42} if '[[INVALID_SESSION_ID]]' in args[-1] else session_fields
        emit({'type':'step_finish',**finish_session,'part':{'reason':'tool-calls' if gap else reason}})
        emit({'type':'text',**session_fields,'part':{'text':text}})
        if '[[STDOUT_HELPER]]' in args[-1].split('<untrusted_scan_evidence>')[0]:
            subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)'])
        if '[[EXIT_FAILURE]]' in args[-1].split('<untrusted_scan_evidence>')[0]: sys.exit(7)
