struct V { int a; };
struct V { int b; };
struct W { int a; };
int g(void) {
    struct W { int b; } w;
    return 0;
}
