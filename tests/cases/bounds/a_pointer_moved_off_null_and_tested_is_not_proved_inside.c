int main(void) {
    int *p = 0;
    int *q = p + 1;
    if (q == 0) {
        return 1;
    }
    *q = 5;
    return 0;
}
