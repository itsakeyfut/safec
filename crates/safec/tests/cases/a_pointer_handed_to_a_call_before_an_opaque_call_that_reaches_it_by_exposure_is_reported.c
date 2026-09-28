void *memset(void *s, int c, int n);
void release_all(void);

int f(int *p) {
    if (p == 0) {
        return 0;
    }
    return (memset(p, 0, 4) != 0) + (release_all(), 0);
}
