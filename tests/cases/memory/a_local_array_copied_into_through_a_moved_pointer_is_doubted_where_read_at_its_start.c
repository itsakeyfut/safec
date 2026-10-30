void *memcpy(void *d, void *s, int n);
int main(void) {
    int x = 1;
    int *m[2];
    int *v = &x;
    memcpy(m + 1, &v, 8);
    int *r = m[0];
    return r == 0;
}
