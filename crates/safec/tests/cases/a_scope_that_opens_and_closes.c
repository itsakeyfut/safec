int outlives(void) {
    int *p;

    {
        int x;
        x = 42;
        p = &x;
    }

    return *p;
}
