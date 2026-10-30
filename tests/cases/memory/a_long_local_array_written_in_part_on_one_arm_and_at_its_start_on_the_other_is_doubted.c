void *memcpy(void *d, void *s, int n);
int c(void);
int main(void) {
    int x = 1;
    int *m[100];
    int *v = &x;
    if (c()) {
        memcpy(m + 1, &v, 8);
    } else {
        m[0] = &x;
    }
    int *r = m[0];
    return r == 0;
}
