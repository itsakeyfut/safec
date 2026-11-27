struct P {
    int x, y;
};
struct P elided[] = {1, 2, 3, 4};
struct P braced[] = {{1, 2}, {3, 4}};
int n = sizeof braced;
