struct I {
    int x;
};
struct O {
    struct I i;
    struct I many[2];
    int y;
};
int f(const struct O *p) {
    return p->i.x + (int)sizeof(p->i) + (int)sizeof(p->many[0]);
}
