void *memcpy(void *d, void *s, int n);
int c(void);
int main(void) {
    int x = 1;
    int *m[100];
    int *v = &x;
    if (c()) {
        m[0] = &x;
    } else {
        memcpy(m + 1, &v, 8);
    }
    int *r = m[0];
    return r == 0;
}
