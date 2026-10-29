int main(void) {
    int x = 1;
    int *m[64];
    m[0] = &x;
    int *r = m[1];
    return r == 0;
}
