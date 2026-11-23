char before;
struct S {
    char c;
    long long x;
} global;
char between;
struct S many[2];
int main(void) {
    struct S local;
    local.x = 1;
    global.x = local.x;
    many[1].x = global.x;
    return 0;
}
