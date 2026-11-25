#define F(x) ((x) + 1)
#define G F

int main(void) { return G(1) - 2; }
