static struct S {
    int a;
} *p;
extern struct U {
    int a;
} *e;
int f(void) {
    static struct V {
        int a;
    } *l;
    return 0;
}
