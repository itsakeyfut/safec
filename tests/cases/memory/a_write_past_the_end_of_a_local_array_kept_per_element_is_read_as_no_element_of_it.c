int main(void) {
    int x = 1;
    int *m[2];
    m[5] = &x;
    int *r = m[0];
    return r == 0;
}
