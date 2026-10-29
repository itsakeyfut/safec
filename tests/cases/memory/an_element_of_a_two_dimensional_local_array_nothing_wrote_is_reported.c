int main(void) {
    int x = 1;
    int *m[2][2];
    m[0][1] = &x;
    int *q = m[1][0];
    return q == 0;
}
