int main(void) {
    const int x = 1;
    int v;
    int *const p = &v;
    x = 2;
    x += 1;
    x++;
    --x;
    p = 0;
    *p = 3;
    return v;
}
