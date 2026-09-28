void release_all(void);

int f(int *p) {
    int x;
    if (p == 0) {
        return 0;
    }
    return (x = p[0]) + (release_all(), 0);
}
