int f(void) {
    int x = 0;
    int *p = &x;
    return x.y + p->y;
}
