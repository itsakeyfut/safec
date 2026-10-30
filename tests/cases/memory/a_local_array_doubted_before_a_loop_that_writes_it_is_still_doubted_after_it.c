int c(void);
int d(void);
int main(void) {
    int *m[2];
    int x = 1;
    if (c()) {
        m[0] = &x;
    }
    while (d()) {
        m[0] = &x;
    }
    int *r = m[0];
    return r == 0;
}
