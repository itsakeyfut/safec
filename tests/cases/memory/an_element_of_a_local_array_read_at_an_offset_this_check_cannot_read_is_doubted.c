int main(int c) {
    int x = 1;
    int *m[3];
    m[1] = &x;
    int *r = m[c];
    return r == 0;
}
