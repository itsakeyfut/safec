int main(void) {
    int x = 1;
    int *m[2];
    m[1] = &x;
    m[0] = &x;
    int *r = m[0];
    return r == 0;
}
