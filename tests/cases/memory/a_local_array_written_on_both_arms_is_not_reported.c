int c(void);
int main(void) {
    int x = 1;
    int *m[2];
    if (c()) {
        m[0] = &x;
    } else {
        m[0] = &x;
    }
    int *r = m[0];
    return r == 0;
}
