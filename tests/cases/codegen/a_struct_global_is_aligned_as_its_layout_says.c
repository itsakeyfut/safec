char before;
struct S {
    char c;
    long long x;
} global;
int main(void) {
    struct S local;
    local.x = 1;
    global.x = local.x;
    return 0;
}
