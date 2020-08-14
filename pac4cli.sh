# /etc/profile.d/pac4cli -- set proxy
http_proxy=http://127.0.0.1:3128
https_proxy=http://127.0.0.1:3128
export http_proxy
export https_proxy

http_host=${http_proxy%:*}
http_host=${http_host##*/}

https_host=${https_proxy%:*}
https_host=${https_host##*/}

_JAVA_OPTIONS="${_JAVA_OPTIONS} -Dhttp.proxyHost=${http_host} -Dhttp.proxyPort=${http_proxy##*:} -Dhttps.proxyHost=${https_host} -Dhttps.proxyPort=${https_proxy##*:}"

export _JAVA_OPTIONS
