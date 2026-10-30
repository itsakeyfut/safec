int c(void);
int main(void) {
    int x = 1;
    int s = 0;
    for (int i = 0; i < 2; i++) {
        int *m[1];
        if (c()) {
            m[0] = &x;
        }
        int *r = m[0];
        s = s + (r != 0);
        m[0] = &x;
    }
    return s;
}
