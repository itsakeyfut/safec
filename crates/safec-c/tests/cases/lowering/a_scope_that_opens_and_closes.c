int outlives(void) {
    int *p;

    {
        int x;
        int y;

        x = 42;
        y = x;
        p = &y;
    }

    return *p;
}
