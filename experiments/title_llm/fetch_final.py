import requests, json, time, os, re

UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
headers = {'User-Agent': UA, 'Referer': 'https://space.bilibili.com/'}

def api_get(url, retries=3):
    for i in range(retries):
        try:
            r = requests.get(url, headers=headers, timeout=15)
            return r.json()
        except Exception as e:
            if i < retries - 1:
                time.sleep(2)
                continue
            return {'code': -1, 'message': str(e)}

# Get user name from page title
def get_page_name(mid):
    r = requests.get(f'https://space.bilibili.com/{mid}', headers=headers, timeout=10)
    r.encoding = 'utf-8'
    m = re.search(r'<title>(.*?)</title>', r.text)
    if m:
        title = m.group(1)
        name = title.split('的个人空间')[0]
        return name
    return f'mid_{mid}'

# Get season archives list  
def get_season_videos(mid, season_id):
    all_videos = []
    pn = 1
    while True:
        url = (f'https://api.bilibili.com/x/polymer/web-space/seasons_archives_list'
               f'?mid={mid}&season_id={season_id}&sort_reverse=false'
               f'&page_size=50&page_num={pn}')
        data = api_get(url)
        if data.get('code') != 0:
            print(f'  API error: code={data["code"]}, msg={data.get("message","")}')
            return []
        items = data.get('data', {}).get('archives', [])
        if not items:
            break
        for a in items:
            all_videos.append({
                'title': a.get('title', ''),
                'duration': a.get('duration', 0),
                'aid': a.get('aid', 0),
                'bvid': a.get('bvid', ''),
            })
        total = data.get('data', {}).get('page', {}).get('total', 0)
        if len(all_videos) >= total:
            break
        pn += 1
        time.sleep(1)
    return all_videos

# Also try /x/space/seasons/list to get season names
def get_seasons_for_user(mid):
    url = f'https://api.bilibili.com/x/space/seasons/list?mid={mid}&page_size=50&page_num=1'
    data = api_get(url)
    if data.get('code') != 0:
        print(f'  seasons list API error: code={data["code"]}')
        return None
    return data.get('data', {})

# Define what to fetch
tasks = [
    (896830, 3149109),
    (20094643, 7304808),
    (20094643, 5598362),
]

for mid, season_id in tasks:
    # Get name
    name = get_page_name(mid)
    safe_name = re.sub(r'[\\/:*?"<>|]', '_', name) if name else f'mid_{mid}'
    print(f'\n=== {name} (mid={mid}), season={season_id} ===')

    # Get season videos
    time.sleep(2)
    videos = get_season_videos(mid, season_id)
    print(f'  Found {len(videos)} videos')

    if not videos:
        print('  Trying series API as fallback...')
        url = f'https://api.bilibili.com/x/series/archives?mid={mid}&series_id={season_id}&current_mid={mid}&only_normal=true&ps=50&pn=1'
        data = api_get(url)
        if data.get('code') == 0:
            for a in data.get('data', {}).get('archives', []):
                videos.append({
                    'title': a.get('title', ''),
                    'duration': a.get('duration', 0),
                    'aid': a.get('aid', 0),
                    'bvid': a.get('bvid', ''),
                })
            print(f'  Found {len(videos)} videos via series API')
    
    if not videos:
        # Try one more fallback: medialist resource
        print('  Trying medialist API as fallback...')
        for type_val in [1, 3, 6]:
            url = f'https://api.bilibili.com/x/v2/medialist/resource/list?type={type_val}&biz_id={season_id}&ps=50'
            data = api_get(url)
            if data.get('code') == 0 and data.get('data', {}).get('media_list'):
                for a in data['data']['media_list']:
                    if a.get('title'):
                        videos.append({
                            'title': a.get('title', ''),
                            'duration': a.get('duration', 0),
                            'aid': a.get('id', 0),
                            'bvid': a.get('bv_id', ''),
                        })
                if videos:
                    print(f'  Found {len(videos)} videos via medialist API')
                    break
            time.sleep(1)
    
    # Save to file
    if videos:
        out_dir = os.path.join(os.path.dirname(__file__), safe_name)
        os.makedirs(out_dir, exist_ok=True)
        
        # Save JSON
        out_json = os.path.join(out_dir, f'season_{season_id}.json')
        with open(out_json, 'w', encoding='utf-8') as f:
            json.dump({
                'streamer': name,
                'mid': mid,
                'season_id': season_id,
                'fetched_at': '2026-06-30',
                'total': len(videos),
                'videos': videos
            }, f, ensure_ascii=False, indent=2)
        
        # Save plain text titles
        out_txt = os.path.join(out_dir, f'season_{season_id}.txt')
        with open(out_txt, 'w', encoding='utf-8') as f:
            for v in videos:
                f.write(v['title'] + '\n')
        
        print(f'  Saved to {out_json}')
        print(f'  Titles preview:')
        for v in videos[:5]:
            print(f'    {v["title"]}')
    else:
        print('  No videos found!')

print('\nDone!')
