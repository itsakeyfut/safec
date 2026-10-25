int *make(void);

int main(void) {
    int *p = make();
    if (p == 0) {
        return 1;
    }
    *p = 1;
    return 0;
}
