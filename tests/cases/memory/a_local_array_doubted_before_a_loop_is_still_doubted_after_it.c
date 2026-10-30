int c(void);
int main(void) {
    int x = 1;
    int *m[2];
    if (c()) {
        m[0] = &x;
    }
    for (int i = 0; i < 2; i++) {
        x = x + 1;
    }
    int *r = m[0];
    return r == 0;
}
