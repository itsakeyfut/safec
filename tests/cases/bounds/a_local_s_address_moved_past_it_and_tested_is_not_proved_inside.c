int main(void) {
    int x = 0;
    int *p = &x;
    int *q = p + 1;
    if (q != 0) {
        *q = 1;
    }
    return x;
}
