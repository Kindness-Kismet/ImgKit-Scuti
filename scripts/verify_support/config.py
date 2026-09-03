# 验证流程的固定资源: ROM 直链、外部工具版本与测试矩阵。

# 小米 11 (venus) 全球版 HyperOS 1 最小包 OS1.0.3.0.UKBMIXM, 实测 5060414718 字节
# 官方五个镜像入口, auto 模式下载前测速选最快
ROM_MIRRORS = {
    "bigota": "https://bigota.d.miui.com/OS1.0.3.0.UKBMIXM/miui_VENUSGlobal_OS1.0.3.0.UKBMIXM_1eb20d7656_14.0.zip",
    "bn": "https://bn.d.miui.com/OS1.0.3.0.UKBMIXM/miui_VENUSGlobal_OS1.0.3.0.UKBMIXM_1eb20d7656_14.0.zip",
    "cdnorg": "https://cdnorg.d.miui.com/OS1.0.3.0.UKBMIXM/miui_VENUSGlobal_OS1.0.3.0.UKBMIXM_1eb20d7656_14.0.zip",
    "aliyun": "https://bkt-sgp-miui-ota-update-alisgp.oss-ap-southeast-1.aliyuncs.com/OS1.0.3.0.UKBMIXM/miui_VENUSGlobal_OS1.0.3.0.UKBMIXM_1eb20d7656_14.0.zip",
    "hugeota": "https://hugeota.d.miui.com/OS1.0.3.0.UKBMIXM/miui_VENUSGlobal_OS1.0.3.0.UKBMIXM_1eb20d7656_14.0.zip",
}
ROM_SIZE = 5_060_414_718
ROM_ZIP_NAME = "miui_VENUSGlobal_OS1.0.3.0.UKBMIXM_1eb20d7656_14.0.zip"

# 镜像测速参数
MIRROR_TEST_BYTES = 8 * 1024 * 1024
MIRROR_TEST_TIMEOUT = 15

# payload-dumper-go: payload 提取基准工具
PDG_ASSET = {
    ("windows", "amd64"): "payload-dumper-go_2.0.2_windows_amd64.tar.gz",
    ("darwin", "amd64"): "payload-dumper-go_2.0.2_darwin_amd64.tar.gz",
    ("darwin", "arm64"): "payload-dumper-go_2.0.2_darwin_arm64.tar.gz",
    ("linux", "amd64"): "payload-dumper-go_2.0.2_linux_amd64.tar.gz",
    ("linux", "arm64"): "payload-dumper-go_2.0.2_linux_arm64.tar.gz",
}
PDG_URL_PREFIX = "https://github.com/ssut/payload-dumper-go/releases/download/2.0.2/"

# erofs-utils (sekaiacg fork): erofs 提取基准工具, 资产名与 tag 不一致故逐个写死
EROFS_ASSET = {
    ("windows", "amd64"): "erofs-utils-v1.8.10-gee46dd74-251217-Cygwin_x86_64.zip",
    ("darwin", "amd64"): "erofs-utils-v1.8.10-gee46dd74-251217-Darwin_x86_64.zip",
    ("darwin", "arm64"): "erofs-utils-v1.8.10-gee46dd74-251217-Darwin_aarch64.zip",
    ("linux", "amd64"): "erofs-utils-v1.8.10-gee46dd74-251217-Linux_x86_64.zip",
    ("linux", "arm64"): "erofs-utils-v1.8.10-gee46dd74-251217-Linux_aarch64.zip",
}
EROFS_URL_PREFIX = "https://github.com/sekaiacg/erofs-tools/releases/download/v1.8.10-251217/"

# erofs 压缩测试矩阵: (算法, 级别), 级别为 None 时不传 --compress-level
# 验证只需覆盖压缩路径, 默认档取最低有效等级以缩短耗时
# lz4 无级别参数; deflate 0 为纯存储不触发压缩路径故取 1
EROFES_TIER_DEFAULT = [
    ("lz4", None),
    ("lz4hc", 0),
    ("lzma", 0),
    ("deflate", 1),
    ("zstd", 1),
]
# 高档档位仅在高压缩场景需要验证时使用
EROFES_TIER_HIGH = [
    ("lz4hc", 12),
    ("lzma", 9),
    ("deflate", 9),
    ("zstd", 15),
]

# ext4/f2fs 打包镜像大小估算: 数据量系数 + 固定余量 + 对齐
FS_SIZE_FACTOR = 1.5
FS_SIZE_MARGIN = 512 * 1024 * 1024
FS_SIZE_ALIGN = 4096

# 目录比对时忽略的名字 (fnmatch 通配): imgkit 与 erofs-utils 的旁车目录/文件
# config 内即 selinux 上下文与权限两份旁车, erofs-utils 侧另有 fs_options
COMPARE_IGNORE_NAMES = {"config", "*_fs_config", "*_file_contexts", "*_fs_options"}

# 各阶段可用名单
ALL_STAGES = ["prepare", "payload", "baseline", "fs"]
ALL_FS_TYPES = ["ext4", "erofs", "f2fs"]
ALL_EROFS_ALGOS = ["lz4", "lz4hc", "lzma", "deflate", "zstd"]
