
const http = require('http');

function post(urlStr) {
    return new Promise((resolve, reject) => {
        const url = new URL(urlStr);
        const options = {
            hostname: url.hostname,
            port: url.port,
            path: url.pathname,
            method: 'POST',
            headers: { 'Content-Type': 'application/json' }
        };

        const req = http.request(options, (res) => {
            resolve({ status: res.statusCode });
            res.resume(); // consume body to free memory
        });

        req.on('error', (e) => reject(e));
        req.end();
    });
}

async function run() {
    const url = 'http://localhost:5050/auth/guest';
    console.log(`Testing Rate Limit for ${url} (Limit: 10/min)`);

    let success = 0;
    let limited = 0;
    let errors = 0;

    for (let i = 1; i <= 15; i++) {
        try {
            const res = await post(url);
            if (res.status === 201) {
                success++;
                console.log(`Req ${i}: 201 OK`);
            } else if (res.status === 429) {
                limited++;
                console.log(`Req ${i}: 429 Too Many Requests`);
            } else {
                console.log(`Req ${i}: Unexpected ${res.status}`);
            }
        } catch (e) {
            errors++;
            console.error(`Req ${i}: Error`, e.message);
        }
    }

    console.log(`\nSummary: Success=${success}, Limited=${limited}, Errors=${errors}`);

    // We expect 10 successes and 5 limited. 
    // However, if the server has been running for a while or other tests ran, 
    // the limit might be hit earlier or later depending on window.
    // But since I just restarted the server, it should be clean.

    if (success === 10 && limited === 5) {
        console.log('VERIFICATION PASSED');
        process.exit(0);
    } else {
        console.log('VERIFICATION FAILED');
        process.exit(1);
    }
}

run();
