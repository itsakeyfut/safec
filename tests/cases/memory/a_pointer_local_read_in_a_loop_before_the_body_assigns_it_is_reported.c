int main(void) {
    int *q;
    int x = 1;
    int *r = 0;
    for (int i = 0; i < 2; i++) {
        r = q;
        q = &x;
    }
    return r == 0;
}
